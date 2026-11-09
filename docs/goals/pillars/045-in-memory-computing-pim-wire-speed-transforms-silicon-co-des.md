<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XLV (45/126); status: DRAFT -->

# Pillar XLV: In-Memory Computing (PIM), Wire-Speed Transforms & Silicon Co-Design

**Breakthroughs 431-440** - tag `ml` - [index](../index.md)


431. **Analog Resistive RAM (ReRAM) Crossbar Accelerators for MVM**  
     Map static neural network weight matrices directly to non-volatile ReRAM / Memristor crossbar arrays. Matrix-Vector Multiplication (MVM) is executed in a single physical step by applying voltages to row lines and reading output currents via Ohm's and Kirchhoff's laws at the columns ($I = V \cdot G$), computing inference with **zero memory bus movement**.

432. **Processing-in-Memory (PIM) for Massive KV-Cache Lookups**  
     Deploy compute-enabled CXL.mem buffers where arithmetic units reside directly within the DRAM logic die (e.g., HBM-PIM / CXL-PIM). Multi-head attention reductions are executed inside the memory chips themselves, slashing memory bus energy consumption by 85%.

433. **Wire-Speed In-Flight Feature Normalization inside FPGA MACs**  
     Compute LayerNorm, RMSNorm, and Z-score standardization directly on streaming telemetry inside the FPGA network ingress MAC. Features arrive at the model's compute core already zero-centered and scaled, stripping the first two computational passes from the forward inference path.

434. **Sub-Microsecond Dynamic Model Swapping via FPGA Partial Reconfiguration**  
     Maintain multiple specialized neural architectures (e.g., high-volatility, low-volatility, crash-regime models) as partial bitstreams in high-speed BRAM. When market regime shifts are detected, the FPGA swaps the active inference accelerator via PCIe MCAP in under **800 nanoseconds** without interrupting network ingress.

435. **Non-Volatile Weight Storage in CXL Persistent Memory (NVDIMM)**  
     Store large multi-gigabyte models in persistent, byte-addressable CXL storage. Cold-booting an inference node requires zero model-loading time: weights are immediately addressable and cache-coherent upon power-up without copying files from disk to RAM.

436. **Hardware-Pipelined Softmax with Max-Subtraction Shift Registers**  
     Implement Softmax entirely in hardware registers using a streaming shift-register pipeline:
     $$P(y_i) = \frac{e^{z_i - z_{\max}}}{\sum_j e^{z_j - z_{\max}}}$$
     As logits stream across the bus, the running maximum is tracked and subtracted in-flight, preventing floating-point overflow and emitting normalized probabilities within 4 cycles of the final logit's arrival.

437. **Direct Tensor Core Scratchpad-to-Scratchpad P2P Clustering**  
     Interconnect adjacent GPU/FPGA compute units via direct on-chip crossbars. Layer $N$ writes its activations directly into the L1/Scratchpad memory of the compute unit assigned to Layer $N+1$, establishing a continuous, assembly-line hardware inference pipeline across the die.

438. **Zero-Overhead Dynamic Batching in Hardware FIFO Queues**  
     When multiple asynchronous data streams require inference, an FPGA scheduler aggregates inputs into micro-batches inside hardware FIFOs using atomic bit-packing. As soon as a FIFO reaches size $B$ or a nanosecond timeout expires, the batch is pulsed to the compute core in a single cycle.

439. **Decoupled Weight-Streaming and Activation-Gathering Engines**  
     Separate memory access into two independent, autonomous hardware engines: Engine A streams weights from memory continuously into register buffers, while Engine B gathers dynamic activations. The tensor ALUs never experience memory-stall bubbles.

440. **Direct FPGA Optical Ingress into Matrix Registers**  
     Connect the optical fiber transceiver directly to the input registers of an FPGA matrix multiplier. Photons hitting the photodiode are converted from serial bits to parallel words and clocked directly into the first layer's multiplier registers, achieving the theoretical physical limit of input-to-compute latency.
