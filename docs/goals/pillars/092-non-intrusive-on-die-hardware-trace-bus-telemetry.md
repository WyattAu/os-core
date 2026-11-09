<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XCII (92/126); status: DRAFT -->

# Pillar XCII: Non-Intrusive On-Die Hardware Trace & Bus Telemetry

**Breakthroughs 901-910** - tag `area` - [index](../index.md)


901. **Zero-Probe-Effect Hardware Telemetry Taps ($\Delta t_{\text{exec}} = 0$)**  
     Abolish software trampolines, Kprobes, and eBPF instrumentation. Telemetry is harvested by dedicated, read-only hardware snooping taps embedded directly into the CPU instruction retirement logic (Reorder Buffer - ROB), memory load/store queues, and register writeback stages. Taps extract execution metrics without adding a single clock cycle of latency or stealing a single execution port.

902. **Direct-to-CXL Hardware Execution Trace Rings**  
     Trace data (branch targets, instruction pointer jumps, capability invocations) streams continuously over dedicated on-chip debug crossbars directly into persistent CXL memory rings. The stream completely bypasses the CPU L1/L2/L3 cache hierarchy, guaranteeing zero instruction-cache or data-cache eviction for operational application workloads.

903. **Real-Time PCIe/CXL TLP Transaction Sniffing in Silicon**  
     The root complex and fabric crossbars incorporate integrated, wire-speed Transaction Layer Packet (TLP) sniffers. Ingress and egress memory read/write requests, split completions, and flow-control credit tokens are logged with sub-nanosecond hardware timestamps into isolated circular buffers without degrading bus throughput.

904. **Sub-Cycle Micro-Architectural Stall Accounting**  
     Hardware Performance Monitors (PMUs) do not rely on coarse, periodic sampling interrupts that cause kernel entry. Dedicated counters continuously record exact cycle penalties for Decoded Stream Buffer (DSB) misses, ROB stalls, store-forwarding blocks, and memory-controller read-queue starvation, projecting live stall matrices into memory.

905. **Non-Intrusive Memory Bus Access Trajectory Capture**  
     Log the temporal sequence of physical memory addresses accessed by an algorithm directly from the memory controller's arbiter lines. The stream is compressed using on-die hardware delta-encoding (saving only $\Delta \text{Addr}$ and $\Delta t$), providing a cycle-accurate historical memory trajectory for forensic analysis.

906. **Line-Rate Network MAC State Vector Streaming**  
     The network PHY and MAC serialize internal physical-layer metrics (PTP clock phase drift, SerDes equalization eye-height, CRC error bursts, FIFO fill levels) directly into a memory-mapped telemetry ring on every clock cycle, diagnosing physical fiber degradation before frame loss occurs.

907. **Hardware-Pipelined Instruction Sequence Compression (Z-Trace)**  
     Raw instruction traces consume gigabytes per second. Synthesize an unrolled, hardware-accelerated branch trace compressor directly into the processor's retirement unit: unconditional branches are omitted, and conditional branches are emitted as single bits ($1 = \text{Taken}, 0 = \text{Not Taken}$), squeezing millions of retired instructions into megabytes of storage.

908. **Dedicated Out-of-Band Hardware Telemetry Network (No-C Interconnect)**  
     On-die telemetry does not share bandwidth with application data on the primary coherent crossbar. Telemetry frames traverse an independent physical **Network-on-Chip (NoC)** routing fabric etched onto the silicon substrate, insulating compute workflows from telemetry congestion.

909. **Hardware-Triggered Selective Trace Windows**  
     Trace logging does not run blindly. The execution tap is armed with single-cycle hardware comparators: tracing activates only when specific physical execution criteria are met (e.g., *Execution latency between Address $A$ and Address $B$ exceeds 45 ns*), logging only the micro-architectural anomalies of interest.

910. **Zero-Overhead Memory Latency Heatmapping in CXL Dies**  
     Memory controllers inside CXL.mem DRAM dies maintain an internal access-frequency and latency histogram across physical memory banks. System profilers read these hardware counters directly to detect memory bank conflicts and NUMA thrashing without running software profilers.
