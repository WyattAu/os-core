<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar III (3/126); status: DRAFT -->

# Pillar III: Heterogeneous Compute & Hardware-Peer Scheduling

**Breakthroughs 19-26** - tag `area` - [index](../index.md)


19. **Actor-Model Heterogeneous Runtime**  
    CPU threads, GPU warps, and FPGA state machines are treated as structurally identical "Actors." They communicate exclusively via lock-free, typed message rings mapped in global memory.

20. **Hardware-Level Directed Acyclic Graph (DAG) Dispatch**  
    The operating system scheduler does not schedule threads; it schedules execution graphs. A completion on an FPGA ring hardware-triggers a GPU compute grid without CPU-side driver intervention.

21. **Unified Compute-Balancing Scheduler**  
    A single mathematical scheduler routes computational kernels across CPU SIMD/AVX-512 engines, GPU streaming multiprocessors, or FPGA systolic arrays based on a real-time, compile-time-verified algorithmic cost model.

22. **Preemptible FPGA Micro-Contexts**  
    Time-slice FPGA hardware pipelines via dynamic partial reconfiguration (DPR) controlled directly by a real-time, verified deadline scheduler.

23. **Cross-Silicon Hardware Semaphores**  
    Implement cross-device atomic primitives (CAS, Fetch-and-Add) that operate coherently across CPU cores, FPGA RTL pipelines, and GPU warps over CXL.cache.

24. **Telemetry-Driven Adaptive Compute Routing**  
    The kernel dynamically shifts a task from CPU to FPGA or GPU based on live hardware performance counters (e.g., L3 miss rates, bus latency, queue depth).

25. **Unified Heterogeneous Fault Propagation**  
    GPU out-of-bounds reads or FPGA parity errors do not trigger opaque driver crashes; they propagate as native, synchronous capability violations to the exact actor that initiated the transaction.

26. **Global Sub-Nanosecond Synchronized Tick Architecture**  
    A unified, cross-silicon clock infrastructure that locks CPU cycle counters, FPGA state machines, and GPU hardware timers to a shared hardware-disciplined clock.
