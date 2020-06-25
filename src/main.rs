use std::collections::HashMap;
use std::env;
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Error, Read, Write};
use std::mem;
use std::os::windows::io::AsRawHandle;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use chrono::prelude::Local;

use tokio::sync::RwLock;
use tokio::runtime::Builder;

use futures::{FutureExt, StreamExt, executor};

use warp::Filter;
use warp::ws::Message;

use serde::{Deserialize, Serialize};

use serde_json::ser::PrettyFormatter;
use serde_json::Serializer;

mod hwinfo;

type Connection = Arc<RwLock<HashMap<usize, tokio::sync::mpsc::UnboundedSender<Result<warp::ws::Message, warp::Error>>>>>;
#[allow(non_camel_case_types)]
type PHANDLER_ROUTINE = Option<unsafe extern "system" fn(CtrlType: u32) -> i32>;

static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
static mut SEMAPHORE_SIGNAL: *mut c_void = 0 as *mut c_void;
static mut SEMAPHORE_DONE: *mut c_void = 0 as *mut c_void;

#[tokio::main]
async fn main() {
    let title = "Jonitor v1.0";
    let input_handle = io::stdin().as_raw_handle();

    let mut mode: u32 = unsafe {
        mem::zeroed()
    };

    unsafe {
        if GetConsoleMode(input_handle, &mut mode) == 0 {
            log!("Unable to get console mode");
        }

        if SetConsoleMode(input_handle, 521u32) == 0 {
            log!("Unable to set console mode");
        }

        let mut v: Vec<u16> = title.encode_utf16().collect();
        v.push(0);

        if SetConsoleTitleW(v.as_ptr()) == 0 {
            log!("Unable to set console title");
        }
    }

    log!("{}", title);

    let should_pause = Arc::new(std::sync::Mutex::new(false));
    let should_pause_clone = should_pause.clone();

    let hwinfo = hwinfo::SharedMemory::new();

    if hwinfo.is_none() {
        log!("Unable to read data from HWiNFO! Make sure HWiNFO is running in Sensors-only mode.");
        pause(mode);
        return;
    }

    let shared_memory = hwinfo.unwrap();

    let mut config;

    let result = read_config();
    match result {
        Ok(c) => config = c,
        Err(e) => {
            println!("{} Error while reading configuration file: {}", Local::now().format("%H:%M:%S"), e);
            pause(mode);
            return;
        }
    };

    let ip_str = format!("{}:{}", config.ip, config.port);

    let ip: SocketAddr = ip_str.parse().unwrap_or_else(|_| {
        log!("Invalid IP address({}). Default value will be used", ip_str);
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 10113)
    });

    if config.polling_interval < 200 {
        log!("Invalid polling interval({}). The value should be bigger than 200. Default value will be used", config.polling_interval);
        config.polling_interval = 1000;
    }

    let polling_interval = config.polling_interval;

    let mut public_dir = env::current_exe().unwrap();

    public_dir.pop();

    if cfg!(debug_assertions) {
        public_dir.pop();
        public_dir.pop();
    }

    public_dir.push("public");

    if config.serve_files {
        if public_dir.exists() && public_dir.is_dir() {
            log!("Serving files from {}", public_dir.to_string_lossy());
        } else {
            config.serve_files = false;
            log!("\"public\" folder does not exist. Static files will not be served");
        }
    } else {
        log!("Static files will not be served");
    }

    let connections: Connection = Connection::default();
    let connections_clone = connections.clone();
    let connections_clone2 = connections.clone();
    let connections = warp::any().map(move || connections.clone());

    let websocket = warp::path("ws").and(warp::ws()).and(connections).and(warp::addr::remote()).map(|ws: warp::ws::Ws, connections, ip| {
        ws.on_upgrade(move |socket| {
            handle_connection(socket, connections, ip)
        })
    });

    let serve_files = config.serve_files;

    let route = warp::any().and_then(move || async move {
        if serve_files {
            Ok(())
        } else {
            Err(warp::reject::not_found())
        }
    }).and(warp::fs::dir(public_dir)).map(|_, file| file);

    let routes = route.or(websocket);

    let (mut server_tx, mut server_rx) = tokio::sync::mpsc::channel::<()>(1);
    let mut server_tx_1 = server_tx.clone();

    let server_result = warp::serve(routes).try_bind_with_graceful_shutdown(ip, async move {
        server_rx.recv().await;
    });

    if server_result.is_err() {
        log!("Unable to bind to {}", ip);
        log!("The error was: {}", server_result.err().unwrap());
        pause(mode);
        return;
    }

    let (_, server) = server_result.unwrap();

    let (tx, rx) = std::sync::mpsc::sync_channel::<()>(0);

    let hwinfo_handler = thread::spawn(move || {
        let mut runtime = Builder::new().build().unwrap();
        loop {
            if rx.try_recv().is_ok() {
                break;
            }

            let result = shared_memory.to_json();

            if result.is_none() {
                log!("Unable to read data from HWiNFO! Was HWiNFO closed?");
                *should_pause_clone.lock().unwrap() = true;
                runtime.block_on(close_connections(4000, "Unable to read data from HWiNFO", &connections_clone));
                server_tx.try_send(()).unwrap();
                break;
            } else {
                runtime.block_on(send_message(result.unwrap(), &connections_clone));
            }

            thread::sleep(Duration::from_millis(config.polling_interval as u64));
        }
    });

    unsafe {
        SEMAPHORE_SIGNAL = CreateSemaphoreW(ptr::null_mut(), 0, 5, ptr::null());
        SEMAPHORE_DONE = CreateSemaphoreW(ptr::null_mut(), 0, 5, ptr::null());

        if !SEMAPHORE_SIGNAL.is_null() || !SEMAPHORE_DONE.is_null() {
            if SetConsoleCtrlHandler(Some(ctrl_handler), 1) == 0 {
                log!("Unable to set console handler");
            }
        } else {
            log!("Unable to set console handler");
        }
    }

    thread::spawn(move || {
        unsafe {
            if WaitForSingleObject(SEMAPHORE_SIGNAL, 0xFFFFFFFFu32) == 0 {
                log!("Closing...");
                tx.send(()).unwrap();
                executor::block_on(close_connections(4001, "Jonitor closed", &connections_clone2));
                server_tx_1.try_send(()).unwrap();
            }
        }
    });

    log!("Started HTTP server on {} with polling interval of {} ms", ip, polling_interval);

    server.await;

    hwinfo_handler.join().unwrap();

    unsafe {
        ReleaseSemaphore(SEMAPHORE_DONE, 1, ptr::null_mut());
    }

    if *should_pause.lock().unwrap() {
        pause(mode);
    } else {
        unsafe {
            SetConsoleMode(io::stdin().as_raw_handle(), mode);
        }
    }
}

fn read_config() -> Result<Config, Error> {
    let mut path = env::current_exe()?;
    path.pop();

    if cfg!(debug_assertions) {
        path.pop();
        path.pop();
    }

    path.push("config.json");

    if path.exists() && path.is_file() {
        let mut file = File::open(path)?;
        let mut content = String::new();

        file.read_to_string(&mut content)?;

        let result = serde_json::from_str::<Config>(&content);

        if result.is_err() {
            log!("Error while parsing configuration file. Default values will be used");
            return Ok(Config {
                ..Default::default()
            });
        }

        Ok(result.unwrap())
    } else {
        let mut file = File::create(path)?;

        let config = Config {
            ..Default::default()
        };

        let buf = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"    ");
        let mut ser = Serializer::with_formatter(buf, formatter);
        config.serialize(&mut ser).unwrap();

        file.write_all(&ser.into_inner())?;

        Ok(config)
    }
}

async fn handle_connection(ws: warp::ws::WebSocket, connections: Connection, ip: Option<SocketAddr>) {
    log!("{} connected", ip.unwrap());

    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);

    let (ws_tx, mut ws_rx) = ws.split();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn(rx.forward(ws_tx).map(|result| {
        if let Err(e) = result {
            log!("WS Error: {}", e);
        }
    }));

    connections.write().await.insert(id, tx);

    while let Some(result) = ws_rx.next().await {
        if let Err(e) = result {
            log!("WS Error: {}", e);
            break;
        }
    }

    connections.write().await.remove(&id);
    log!("{} disconnected", ip.unwrap());
}

async fn send_message(message: String, connections: &Connection) {
    for (_, tx) in connections.read().await.iter() {
        let _ = tx.send(Ok(Message::text(&message)));
    }
}

async fn close_connections(code: u16, reason: &'static str, connections: &Connection) {
    for (_, tx) in connections.read().await.iter() {
        let _ = tx.send(Ok(Message::close_with(code, reason)));
    }
}

fn pause(mode: u32) {
    print!("Press any key to continue...");
    io::stdout().flush().unwrap();
    io::stdin().read(&mut [0]).unwrap();
    unsafe {
        SetConsoleMode(io::stdin().as_raw_handle(), mode);
    }
}

#[derive(Serialize, Deserialize)]
struct Config {
    ip: String,
    port: i32,
    polling_interval: i32,
    serve_files: bool
}

impl Default for Config {
    fn default() -> Config {
        Config {
            ip: "0.0.0.0".to_string(),
            port: 10113,
            polling_interval: 1000,
            serve_files: true
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        let time = Local::now();
        writeln!(io::stdout(), "{} {}", time.format("%H:%M:%S"), format_args!($($arg)*)).unwrap();
    };
}

#[link(name = "user32")]
extern "system" {
    fn SetConsoleCtrlHandler(HandlerRoutine: PHANDLER_ROUTINE , Add: i32) -> i32;
    fn CreateSemaphoreW(lpSemaphoreAttributes: *mut SECURITY_ATTRIBUTES, lInitialCount: i32, lMaximumCount: i32, lpName: *const u16) -> *mut c_void;
    fn ReleaseSemaphore(hSemaphore: *mut c_void, lReleaseCount: i32, lpPreviousCount: *mut i32) -> i32;
    fn WaitForSingleObject(hHandle: *mut c_void, dwMilliseconds: u32) -> u32;
    fn GetConsoleMode(hConsoleHandle: *mut c_void, lpMode: *mut u32) -> i32;
    fn SetConsoleMode(hConsoleHandle: *mut c_void, dwMode: u32) -> i32;
    fn SetConsoleTitleW(lpConsoleTitle: *const u16) -> i32;
}

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> i32 {
    if ctrl_type == 0 || ctrl_type == 2 || ctrl_type == 6 {
        ReleaseSemaphore(SEMAPHORE_SIGNAL, 1, ptr::null_mut());

        WaitForSingleObject(SEMAPHORE_DONE, 0xFFFFFFFFu32);

        return 1;
    }

    0
}

#[allow(non_snake_case)]
#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: u32,
    pub lpSecurityDescriptor: *mut c_void,
    pub bInheritHandle: i32,
}