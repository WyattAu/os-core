<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XXXIII (33/126); status: DRAFT -->

# Pillar XXXIII: Command Submission, GPU Scheduling & Driverless Shaders

**Breakthroughs 311-320** - tag `gfx` - [index](../index.md)


311. **Total Eradication of User-Mode Drivers (UMD) & Kernel-Mode Drivers (KMD)**  
     Abolish the traditional display driver stack (Vulkan loader, Mesa, proprietary `.ko` modules). Applications compile GPU instructions (AMD RDNA ISA, NVIDIA SASS, or custom VLIW) directly into binary buffers and write them to hardware command rings via `MOVDIR64B` doorbells.

312. **Sub-Microsecond GPU Hardware Preemption**  
     Traditional GPUs require milliseconds to drain draw calls before context-switching. Configure the GPU's internal hardware compute scheduler (e.g., AMD MEC - MicroEngine Compute) to support preemption at the instruction boundary, allowing latency-critical rendering passes to preempt long-running compute jobs in under 200 nanoseconds.

313. **Ahead-of-Time (AOT) Mathematically Verified Shader Compilation**  
     Shaders are not delivered as SPIR-V or HLSL and compiled at runtime by an unpredictable JIT compiler. Shaders are written in your custom systems language, formally verified for bounds, arithmetic safety, and execution limits in Lean 4, and compiled AOT directly to native GPU binary ISA during project build.

314. **Autonomous GPU-Side Dynamic Work Generation**  
     Eliminate the CPU from the draw-call loop. GPU compute warps populate command submission rings (indirect draw/dispatch buffers) located directly in high-speed GPU on-chip shared memory, executing hierarchical scene traversals without a single host CPU wake-up.

315. **Zero-Overhead Bindless Resource Arrays via Direct Physical Pointers**  
     Abolish descriptor sets, root signatures, and resource bindings. Textures, buffers, and acceleration structures are addressed as raw 64-bit physical memory pointers passed directly in GPU scalar registers, turning texture lookups into standard dereference instructions.

316. **Unified Heterogeneous Shading across CPU SIMD, GPU Warps, and FPGA Logic**  
     A single shader function declaration compiles simultaneously to AVX-512 vector code, GPU compute warps, and FPGA systolic pipelines. The kernel's workload balancer routes compute tiles dynamically across whatever compute silicon has idle cache capacity.

317. **Hardware-Enforced Shader Execution Deadlines**  
     Shaders carry an architectural cycle-counter limit written to hardware watchdog registers. If a shader enters an infinite ray-traversal loop or suffers a numerical divergence, the GPU’s hardware scheduler cleanly isolates and terminates the offending thread without hanging the system or requiring a full GPU reset.

318. **Direct PCIe P2P Texture Streaming from NVMe to GPU VRAM**  
     Textures stream directly from raw NVMe flash sectors to GPU High-Bandwidth Memory (HBM) over PCIe switches using GPUDirect Storage protocols without bouncing through host RAM or passing through a virtual file system.

319. **Wavefront/Warp-Level Cooperative Matrix Work-Stealing**  
     Implement lock-free work-stealing queues directly in GPU Subgroup/Warp Scratchpad Memory. If a warp finishes ray-tracing a tile early, it automatically pops rays from adjacent busy warps within the same compute unit, maintaining 100% vector ALU saturation.

320. **Hardware Register-Pressure-Adaptive Thread Sizing**  
     The compiler dynamically measures register occupancy. Instead of failing compilation when register pressure spikes, the runtime dynamically re-tessellates the wave size (e.g., switching dynamically between Wave32 and Wave64 on RDNA) to maintain maximal occupancy without spilling registers to scratch memory.
