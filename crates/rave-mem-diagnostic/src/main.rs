use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};

struct LiveBuf {
    id: u64,
    bytes: u64,
    label: String,
    usage: u32,
    created_at: Instant,
    backtrace: String,
}

static NEXT_ID: Mutex<u64> = Mutex::new(0);
static INVENTORY: Mutex<Option<HashMap<u64, LiveBuf>>> = Mutex::new(None);

fn track_alloc(bytes: u64, label: &str, usage: u32) -> u64 {
    let id = {
        let mut n = NEXT_ID.lock().unwrap();
        let v = *n;
        *n += 1;
        v
    };
    let bt = Backtrace::force_capture().to_string();
    let mut inv = INVENTORY.lock().unwrap();
    if inv.is_none() {
        *inv = Some(HashMap::new());
    }
    inv.as_mut().unwrap().insert(
        id,
        LiveBuf {
            id,
            bytes,
            label: label.to_string(),
            usage,
            created_at: Instant::now(),
            backtrace: bt,
        },
    );
    id
}

fn track_destroy(id: u64) {
    let mut inv = INVENTORY.lock().unwrap();
    if let Some(m) = inv.as_mut() {
        m.remove(&id);
    }
}

fn emit_inventory(t: u64, sys: &mut System, pid: Pid) {
    sys.refresh_process_specifics(pid, ProcessRefreshKind::everything());
    let rss_mb = sys
        .process(pid)
        .map(|p| p.memory() as f64 / 1_048_576.0)
        .unwrap_or(-1.0);

    let inv = INVENTORY.lock().unwrap();
    let m = match inv.as_ref() {
        Some(m) => m,
        None => {
            println!("[mem-diag@{t}s] rss={rss_mb:.2}MB (no live buffers)");
            return;
        }
    };

    let mut rows: Vec<&LiveBuf> = m.values().collect();
    rows.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    let total_bytes: u64 = rows.iter().map(|r| r.bytes).sum();

    println!(
        "[mem-diag@{t}s] rss={:.2}MB live_buffers={} live_bytes={:.2}MB",
        rss_mb,
        rows.len(),
        total_bytes as f64 / 1_048_576.0,
    );

    let now = Instant::now();
    for r in rows.iter().take(10) {
        let age = now.duration_since(r.created_at).as_secs();
        println!(
            "  #{id} {mb:.3}MB usage=0x{u:x} label=\"{lbl}\" age={age}s",
            id = r.id,
            mb = r.bytes as f64 / 1_048_576.0,
            u = r.usage,
            lbl = r.label,
        );
        let bt_lines: Vec<&str> = r.backtrace.lines().take(12).collect();
        for line in bt_lines {
            println!("      {}", line.trim());
        }
    }
    if rows.len() > 10 {
        let tail_bytes: u64 = rows[10..].iter().map(|r| r.bytes).sum();
        println!(
            "  ...{} more totaling {:.2}MB",
            rows.len() - 10,
            tail_bytes as f64 / 1_048_576.0,
        );
    }
}

fn main() {
    let run_secs: u64 = std::env::var("MEM_DIAG_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::SECONDARY,
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no wgpu adapter available — install mesa-vulkan-drivers for lavapipe");

    let info = adapter.get_info();
    println!(
        "[mem-diag] adapter name={:?} backend={:?} device_type={:?}",
        info.name, info.backend, info.device_type
    );

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("rave-mem-diagnostic-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .expect("request_device failed");
    let _ = queue;

    let mut sys = System::new_with_specifics(
        RefreshKind::everything().with_processes(ProcessRefreshKind::everything()),
    );
    let pid = Pid::from(std::process::id() as usize);

    let start = Instant::now();
    let mut last_emit = start;
    let mut alloc_counter: u64 = 0;

    println!("[mem-diag] running for {run_secs}s — replicating Bevy buffer-replace pattern");
    while start.elapsed().as_secs() < run_secs {
        // Simulate a Bevy-shaped workload: allocate a batch of storage buffers
        // sized like clustered-forward cluster storage grows with scene
        // objects. We DO NOT call destroy() on the previous batch — this is
        // the replace-without-destroy pattern the browser probe couldn't see
        // the source of. Here, each alloc captures a full Rust backtrace so
        // the source is inline in the output.
        for size_kb in [64u64, 128, 256, 512, 1024] {
            let size = size_kb * 1024;
            let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            let label = format!("stress-{}kb-#{}", size_kb, alloc_counter);
            let desc = wgpu::BufferDescriptor {
                label: Some(&label),
                size,
                usage,
                mapped_at_creation: false,
            };
            let _buf = device.create_buffer(&desc);
            track_alloc(size, &label, usage.bits());
            alloc_counter += 1;
            // Note: _buf is dropped here. On native wgpu Vulkan backend,
            // Drop calls destroy() correctly (unlike WebGPU wasm backend).
            // To reproduce a leak-shaped growth on native we'd need to hold
            // references — for this first pass, we just observe what the
            // native Drop path does.
        }

        // Also track destroy events by wrapping periodically: every 20 iters,
        // "destroy" the oldest 5 by removing from inventory. This lets us
        // exercise the tracker's decrement path.
        if alloc_counter % 20 == 0 {
            let ids_to_destroy: Vec<u64> = {
                let inv = INVENTORY.lock().unwrap();
                let m = inv.as_ref().unwrap();
                let mut v: Vec<u64> = m.keys().copied().collect();
                v.sort();
                v.into_iter().take(5).collect()
            };
            for id in ids_to_destroy {
                track_destroy(id);
            }
        }

        if last_emit.elapsed() >= Duration::from_secs(5) {
            let t = start.elapsed().as_secs();
            emit_inventory(t, &mut sys, pid);
            last_emit = Instant::now();
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    println!("[mem-diag] final snapshot:");
    emit_inventory(start.elapsed().as_secs(), &mut sys, pid);
    println!(
        "[mem-diag] done · total_allocs={} · runtime={}s",
        alloc_counter,
        start.elapsed().as_secs()
    );
}
