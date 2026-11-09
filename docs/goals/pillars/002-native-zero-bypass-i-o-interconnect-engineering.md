<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar II (2/126); status: DRAFT -->

# Pillar II: Native Zero-Bypass I/O & Interconnect Engineering

**Breakthroughs 11-18** - tag `K2` - [index](../index.md)


11. **Pure Doorbell-Driven Execution (No Software Interrupts)**  
    Abolish the `syscall` instruction, software interrupts, and kernel traps. All system interactions occur via lock-free submission and completion queues updated via memory-mapped I/O (MMIO) doorbells.

12. **Native Wire-to-HBM Direct Streaming**  
    Network and sensor interfaces parse packet headers at line rate in an FPGA front-end; payloads stream directly into GPU High-Bandwidth Memory (HBM) over PCIe/CXL P2P DMA without traversing host RAM or CPU caches.

13. **Deterministic Hardware Timestamp Ingestion**  
    Every queue pair descriptor automatically receives a sub-nanosecond hardware timestamp injected by the PCIe root complex or FPGA ingress pipeline, guaranteeing total causal ordering across all system events.

14. **Autonomous Peer-to-Peer DMA Mesh**  
    Devices (NIC $\leftrightarrow$ FPGA $\leftrightarrow$ NVMe $\leftrightarrow$ GPU) arbitrate and execute DMA transactions directly among themselves via an IOMMU capability matrix, completely removing the CPU from the data plane.

15. **Zero-SerDe In-Memory Wire Encodings**  
    OS data structures are byte-aligned to PCIe TLP and network packet limits. Moving data across the wire or the bus is a typed cast over raw memory, eliminating all serialization and deserialization overhead.

16. **Direct Queue Mapping to Applications**  
    SR-IOV and SVA (Shared Virtual Addressing) taken to the extreme: the kernel only configures hardware permissions at initialization. Application threads map hardware ring buffers directly, making "bypass" the standard OS architecture.

17. **Hardware-Paced In-Flight Flow Control**  
    Eliminate software-level buffer management and TCP window accounting. Ring buffers use PCIe credit counters and backpressure signals to pace data transfers at the silicon level.

18. **Core-Partitioned Asynchronous Event Loops**  
    Replace OS thread preemption with dedicated polling cores executing hardware-optimized `UMWAIT`/`TPAUSE` loops, dynamically adjusting power states while maintaining single-digit nanosecond wake-up latencies.
