//! Combined snapshot of all system information.
//!
//! This is the "give me everything" command. It outputs all available
//! system information in a single JSON object. Useful for:
//!
//! - Saving system state for later analysis
//! - Comparing systems ("why does this board work but that one doesn't?")
//! - Automated inventory collection
//!
//! Note: This always outputs JSON. If you want text output, run the
//! individual subcommands instead.
//!
//! Currently only includes subcommands that have been converted to no_std.

use crate::cli::GlobalOptions;
use crate::io::KbToBytes;
use crate::json::{StreamingJsonWriter, begin_kv_output_streaming};

/// Entry point for `kv snapshot` subcommand.
pub fn run(opts: &GlobalOptions) -> i32 {
    let pretty = opts.pretty;
    let verbose = opts.verbose;

    let mut w = begin_kv_output_streaming(pretty, "snapshot");

    w.field_object("data");

    // CPU info
    #[cfg(feature = "cpu")]
    if let Some(info) = crate::cpu::CpuInfo::read() {
        w.key("cpu");
        write_cpu_json(&mut w, &info, verbose);
    }

    // Memory info
    #[cfg(feature = "mem")]
    if let Some(info) = crate::mem::MemInfo::read() {
        w.key("mem");
        write_mem_json(&mut w, &info, verbose, opts.human);
    }

    // Mount points
    #[cfg(feature = "mounts")]
    crate::mounts::write_snapshot(&mut w, verbose);

    // PCI devices
    #[cfg(feature = "pci")]
    crate::pci::write_snapshot(&mut w, verbose);

    // USB devices
    #[cfg(feature = "usb")]
    crate::usb::write_snapshot(&mut w, verbose);

    // Block devices
    #[cfg(feature = "block")]
    crate::block::write_snapshot(&mut w, verbose);

    // Thermal sensors
    #[cfg(feature = "thermal")]
    crate::thermal::write_snapshot(&mut w, verbose);

    // Power supplies
    #[cfg(feature = "power")]
    crate::power::write_snapshot(&mut w, verbose);

    // Network interfaces
    #[cfg(feature = "net")]
    crate::net::write_snapshot(&mut w, verbose);

    // Device tree (ARM/AArch64/RISC-V only)
    #[cfg(all(feature = "dt", any(target_arch = "arm", target_arch = "aarch64", target_arch = "riscv64", target_arch = "powerpc64", target_arch = "mips")))]
    crate::dt::write_snapshot(&mut w, verbose);

    w.end_field_object();
    w.end_object();
    w.finish();

    0
}

/// Write CPU info as a JSON object (without the key).
#[cfg(feature = "cpu")]
fn write_cpu_json(w: &mut StreamingJsonWriter, info: &crate::cpu::CpuInfo, verbose: bool) {
    use crate::fields::cpu as f;

    w.begin_object();
    w.field_u64(f::LOGICAL_CPUS, info.logical_cpus as u64);
    w.field_str_opt(f::MODEL_NAME, info.model_name.as_ref().map(|s| s.as_str()));
    w.field_str_opt(f::VENDOR_ID, info.vendor_id.as_ref().map(|s| s.as_str()));
    w.field_u64_opt(f::SOCKETS, info.sockets.map(|v| v as u64));
    w.field_u64_opt(f::CORES_PER_SOCKET, info.cores_per_socket.map(|v| v as u64));
    w.field_str_opt(f::ISA, info.isa.as_ref().map(|s| s.as_str()));
    w.field_str_opt(f::MMU, info.mmu.as_ref().map(|s| s.as_str()));

    if verbose {
        w.field_u64_opt(f::CPU_FAMILY, info.cpu_family.map(|v| v as u64));
        w.field_u64_opt(f::MODEL, info.model.map(|v| v as u64));
        w.field_u64_opt(f::STEPPING, info.stepping.map(|v| v as u64));
        if let Some(mhz_x100) = info.cpu_mhz_x100 {
            let mut buf: crate::stack::StackString<16> = crate::stack::StackString::new();
            let whole = mhz_x100 / 100;
            let frac = mhz_x100 % 100;
            let mut itoa_buf = itoa::Buffer::new();
            buf.push_str(itoa_buf.format(whole));
            buf.push('.');
            if frac < 10 {
                buf.push('0');
            }
            buf.push_str(itoa_buf.format(frac));
            w.field_str(f::CPU_MHZ, buf.as_str());
        }
        w.field_str_opt(f::CACHE_SIZE, info.cache_size.as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::ARCHITECTURE, info.architecture.as_ref().map(|s| s.as_str()));
    }

    w.end_object();
}

/// Write memory info as a JSON object (without the key).
#[cfg(feature = "mem")]
fn write_mem_json(w: &mut StreamingJsonWriter, info: &crate::mem::MemInfo, verbose: bool, human: bool) {
    use crate::fields::mem as f;

    w.begin_object();

    if human {
        w.field_str_opt(f::MEM_TOTAL, info.mem_total_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::MEM_FREE, info.mem_free_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::MEM_AVAILABLE, info.mem_available_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::SWAP_TOTAL, info.swap_total_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
        w.field_str_opt(f::SWAP_FREE, info.swap_free_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));

        if verbose {
            w.field_str_opt(f::BUFFERS, info.buffers_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::CACHED, info.cached_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::SWAP_CACHED, info.swap_cached_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::SHMEM, info.shmem_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::SRECLAIMABLE, info.sreclaimable_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::SUNRECLAIM, info.sunreclaim_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::DIRTY, info.dirty_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
            w.field_str_opt(f::WRITEBACK, info.writeback_kb.map(|v| crate::io::format_human_size(v.kb())).as_ref().map(|s| s.as_str()));
        }
    } else {
        w.field_u64_opt(f::MEM_TOTAL_KB, info.mem_total_kb);
        w.field_u64_opt(f::MEM_FREE_KB, info.mem_free_kb);
        w.field_u64_opt(f::MEM_AVAILABLE_KB, info.mem_available_kb);
        w.field_u64_opt(f::SWAP_TOTAL_KB, info.swap_total_kb);
        w.field_u64_opt(f::SWAP_FREE_KB, info.swap_free_kb);

        if verbose {
            w.field_u64_opt(f::BUFFERS_KB, info.buffers_kb);
            w.field_u64_opt(f::CACHED_KB, info.cached_kb);
            w.field_u64_opt(f::SWAP_CACHED_KB, info.swap_cached_kb);
            w.field_u64_opt(f::SHMEM_KB, info.shmem_kb);
            w.field_u64_opt(f::SRECLAIMABLE_KB, info.sreclaimable_kb);
            w.field_u64_opt(f::SUNRECLAIM_KB, info.sunreclaim_kb);
            w.field_u64_opt(f::DIRTY_KB, info.dirty_kb);
            w.field_u64_opt(f::WRITEBACK_KB, info.writeback_kb);
        }
    }

    w.end_object();
}
