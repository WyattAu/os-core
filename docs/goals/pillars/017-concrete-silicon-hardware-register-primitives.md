<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XVII (17/126); status: DRAFT -->

# Pillar XVII: Concrete Silicon & Hardware Register Primitives

**Breakthroughs 151-160** - tag `K2` - [index](../index.md)


151. **Atomic 64-Byte Direct Bus Enqueue via `MOVDIR64B`**  
     Bypass the CPU store buffer and L1/L2 cache allocation entirely for work submission. Use the x86 `MOVDIR64B` instruction to write a complete 64-byte descriptor atomically across the PCIe root complex to device MMIO in a single memory transaction, eliminating fragmented split-TLP writes.

152. **Non-Blocking Shared Work Dispatch via `ENQCMD`/`ENQCMDS` with PASID**  
     Use `ENQCMD` to submit descriptors directly from unprivileged rings to hardware work queues. The instruction passes the 20-bit Process Address Space ID (PASID) in the TLP header, allowing the IOMMU to validate ownership in hardware without a kernel system call.

153. **Low-Power Nanosecond Polling via `UMONITOR` / `UMWAIT`**  
     Replace active cache-line spinning loops with the user-level monitor pair:
     ```rust
     unsafe {
         core::arch::x86_64::_umonitor(target_cacheline_ptr);
         core::arch::x86_64::_umwait(0 /* C0.1 state */, timeout_tsc);
     }
     ```
     The core halts execution pipeline activity and drops into a low-power, sub-nanosecond wake-up state until a write snoops the monitored cache line.

154. **TLP Processing Hints (TPH) Cache Injection Injection Flags**  
     When synthesizing PCIe TLPs from the FPGA, set the 2-bit **TPH (TLP Processing Hints)** field and the 8-bit **Steering Tag (ST)** in the PCIe header. This instructs the host CPU memory controller to inject incoming DMA payloads directly into the L2/L3 cache of a specific CPU APIC ID, bypassing DDR5 main memory entirely.

155. **MSR Hardware Prefetcher Neutralization for Zero-Variance Cycles**  
     At boot, configure Model-Specific Register `0x1A4` (`MSR_IA32_FEAT_CTL` / `MSR_PREFETCH_CONTROL`) on dedicated low-latency cores:
     * Bit 0: Disable L2 hardware prefetcher.
     * Bit 1: Disable L2 adjacent cache line prefetcher.
     * Bit 2: Disable DCU (L1 D-Cache) streamer prefetcher.
     * Bit 3: Disable DCU IP-based prefetcher.  
     This eliminates execution cycle jitter caused by unpredictable hardware prefetch collisions on the memory bus.

156. **System Management Interrupt (SMI) Neutralization**  
     Clear the SMI Enable bit (`SMI_EN`, offset `0x30` in the Intel/AMD Power Management I/O space) or write directly to the chipset’s `SMI_CTRL` register during the stage-0 boot sequence. This disables System Management Mode (SMM) runtime execution, permanently preventing BIOS firmware from hijacking CPU cycles.

157. **Direct MSI-X APIC Target Vectoring**  
     Configure device MSI-X vectors directly by programming the MSI-X Table Entry BAR:
     * Lower/Upper Address: Target local APIC memory-mapped physical address (`0xFEE00000`).
     * Message Data: Vector index mapped directly to the dedicated interrupt service core, with the Delivery Mode set to `Fixed` and Destination Mode set to `Physical`, avoiding OS interrupt-balancer indirection.

158. **PCIe Extended Tag Support (10-bit Tags)**  
     Enable 10-bit Extended Tags in the PCIe Device Control 2 Register (Bit 12) for all endpoints. This expands the maximum number of outstanding, in-flight non-posted PCIe memory read requests from 256 to 1024, saturating PCIe Gen 5/6 bus bandwidth over deep latency queues.

159. **Dynamic Clock Lockdown via `IA32_PERF_CTL`**  
     Lock every compute core to its absolute base frequency by writing the unhalted multiplier directly to `MSR_IA32_PERF_CTL` (bits 15:8) and setting the Turbo Mode Disable bit in `MSR_IA32_MISC_ENABLE` (bit 38). This eradicates frequency-transition latencies (P-state shifts).

160. **Direct Dynamic Reconfiguration via PCIe MCAP**  
     Interface directly with the FPGA’s **Media Configuration Access Port (MCAP)** inside the PCIe extended capability space (Capability ID `0x000B`). Write bitstream chunks directly to MCAP registers over PCIe MMIO, dynamically reprogramming FPGA accelerators without cycling host bus links.
