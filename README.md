# Jonitor
Jonitor is a Windows console application written in Rust. It reads data from [HWiNFO](https://www.hwinfo.com) and send them through WebSocket. It also starts a HTTP server to serve static files.


## Download
Download from the [HWiNFO forum thread](https://www.hwinfo.com/forum/threads/jonitor-v1-0.6511/).

## Data structure (JSON)
```
{
    cpus: [
        {
            name: String,
            cores: [
                {
                    coreNo: i32,
                    threads: [
                        {
                            threadNo: i32,
                            clock: [f64, f64],
                            usage: f64
                        },
                        {
                            ... // up to 2 threads
                        }
                    ],
                    voltage: f64,
                    temperature: [f64, f64]
                    thermalThrottle: [i32, i32],
                    powerThrottle: [i32, i32]
                }
            ],
            usage: f64,
            temperature: [f64, f64],
            power: [f64, f64],
            maxClock: f64,
            maxVoltage: f64
        }
    ],
    gpus: [
        {
            name: String,
            temperature: [f64, f64],
            voltage: [f64, f64],
            power: [f64, f64],
            coreClock: [f64, f64],
            memoryClock: [f64, f64],
            coreUsage: f64,
            memoryUsage: f64,
            memoryUnit: String,
            coreLoad: f64,
            memoryLoad: f64,
            powerLimit: [i32, i32],
            thermalLimit: [i32, i32],
            fan: [f64, f64]
        },
        {
            ... // up to 2 GPUs
        }
    ],
    memoryUsed: f64,
    memoryAvailable: f64,
    virtualMemoryCommited: f64,
    virtualMemoryAvailable: f64,
    memoryClock: f64,
    memoryClockRatio: f64,
    memoryTimings: String,
    processCount: u32,
    threadCount: u32,
    handleCount: u32,
    temperatureUnit: String,
    pollTime: i64
}
// [f64/i32, f64/i32] = [current value, maximum value]
```

## Configuration file options (config.json)
```
{
    "ip": "0.0.0.0", // the IP address the server listens on
    "port": 10113, // the port the server listens on
    "polling_interval": 1000, // time between data is read from HWiNFO in milliseconds
    "serve_files": true // whether to serve static files in public/
}
```

**NOTE:** The file `hwinfo.rs` is not available since open source applications using HWiNFO shared memory cannot release the specification.