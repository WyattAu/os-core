<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar CIX (109/126); status: DRAFT -->

# Pillar CIX: Real-Time Quantum Error Correction (QEC) Decoders in Silicon

**Breakthroughs 1071-1080** - tag `area` - [index](../index.md)


1071. **Sub-150-Nanosecond Union-Find QEC Decoders in Hardware Logic**  
      Synthesize the Union-Find topological decoder directly into combinatorial FPGA logic blocks. Syndrome graph clustering and peeling operations execute on a parallel systolic hardware grid, finding correction chains for surface codes (up to distance $d = 7$) in **under 150 nanoseconds**.

1072. **Systolic Minimum-Weight Perfect Matching (MWPM) Decoders**  
      For higher decoding accuracy: implement parallel Kolmogorov Blossom algorithms or localized systolic MWPM directly across GPU Tensor Cores. Evaluates boundary matching across 3D space-time syndrome graphs concurrently, keeping pace with multi-megahertz syndrome measurement cycles.

1073. **In-Scratchpad Neural-Network QEC Decoders**  
      Execute real-time syndrome decoding using ultra-quantized (INT4/FP8) Graph Neural Networks (GNNs) or 1D convolutional decoders running directly in on-chip SRAM/scratchpads, predicting logical Pauli corrections ($\hat{X}, \hat{Z}$) in **under 80 nanoseconds**.

1074. **Pipelined Surface-Code Syndrome Extraction Graphs**  
      X- and Z-type stabilizer measurements (ancilla qubit parity cycles) are pipelined directly into streaming hardware FIFO queues. Stabilizer values ($S_i = \pm 1$) are decoded differentially against historical epochs, isolating syndrome defect locations on-the-fly.

1075. **Bivariate Bicycle and Quantum LDPC (qLDPC) Decoders in Silicon**  
      Implement iterative belief-propagation and Ordered Statistics Decoding (OSD) for high-rate Quantum Low-Density Parity-Check (qLDPC) codes in FPGA DSP blocks, achieving fault-tolerant quantum memory with an order of magnitude fewer physical qubits than standard surface codes.

1076. **Continuous In-Situ Error Model Calibration**  
     The QEC decoder continuously updates its internal syndrome edge weights ($w_e = \ln \frac{1 - p_e}{p_e}$) based on live physical error rate telemetry. If a specific physical qubit degrades (e.g., through a TLS defect jump), the decoder adjusts matching weights dynamically without software recompilation.

1077. **Hardware Measurement Error Mitigation (MEM) Inversion**  
      Pre-multiply raw measurement outcome probability vectors by the inverse assignment matrix ($\mathbf{A}^{-1}$) using unrolled SIMD dot products in hardware, eliminating readout fidelity asymmetry ($|0\rangle \to |1\rangle$ vs. $|1\rangle \to |0\rangle$) in real time.

1078. **Classical Pauli Frame Tracking in Silicon Registers**  
      Abolish executing physical physical Pauli correction pulses ($X$ and $Z$ gates) post-decoding. The classical controller simply updates a 2-bit **Pauli Frame Register** for each logical qubit in software/silicon. Subsequent Clifford gates are commuted through the Pauli frame in classical logic, saving physical qubit gate budgets completely.

1079. **Single-Shot 3D Gauge Color-Code Decoding**  
      Synthesize decoders for 3D gauge color codes that exploit single-shot error correction: errors are diagnosed and corrected in a single measurement round without requiring repeated stabilizer extraction cycles, slashing QEC cycle latency by $10\times$.

1080. **Hardware-Enforced Magic State Distillation Sequencing**  
      Manage the execution pipeline of fault-tolerant $T$-state factories (Bravyi-Kitaev distillation routines) directly in hardware work rings. Purified magic states are prepared and injected into target computational registers with zero idle pipeline waiting.
