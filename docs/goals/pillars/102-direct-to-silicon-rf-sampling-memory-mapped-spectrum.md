<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar CII (102/126); status: DRAFT -->

# Pillar CII: Direct-to-Silicon RF Sampling & Memory-Mapped Spectrum

**Breakthroughs 1001-1010** - tag `area` - [index](../index.md)


1001. **Direct RF-Sampling ADC/DAC Streaming to CXL.mem (Zero-Driver Ingress)**  
      Bypass intermediate SDR software drivers and network encapsulation. Direct RF-sampling converters (operating up to 64 GSPS directly in the L/S/C/X bands) stream raw, unpacketized In-Phase and Quadrature (I/Q) words directly into unified persistent CXL memory rings via PCIe/CXL P2P DMA, achieving **under 15 nanoseconds** of analog-to-memory ingress latency.

1002. **First-Class Memory-Mapped I/Q Sample Structures**  
      Abolish opaque sample byte buffers. Complex baseband signals are exposed as natively typed, cache-line-aligned SIMD structures in the Single Address Space:
      ```rust
      #[repr(C, align(64))]
      pub struct IqChunk<const SAMPLES: usize> {
          pub i_channels: [i16; SAMPLES], // Aligned for AVX-512 / Tensor Core dot products
          pub q_channels: [i16; SAMPLES],
          pub timestamp_ptp: u64,          // Sub-picosecond hardware timestamp
          pub center_freq_hz: u64,        // Carrier metadata
          pub gain_code: u16,
      }
      ```

1003. **Single-Cycle Digital Down-Conversion (DDC) via FPGA CORDIC Pipelines**  
      Carrier frequency mixing, decimation, and quadrature shifting occur entirely in an unrolled, pipelined Coordinate Rotation Digital Computer (CORDIC) block in FPGA logic. Digital Down-Conversion and Digital Up-Conversion (DUC) execute in a continuous streaming pipeline, processing 64 complex samples per clock tick on a 500 MHz clock.

1004. **Direct Electro-Optic Modulation via Silicon Photonics Ingress**  
      For ultra-wideband radar or microwave photonics: incoming RF signals directly modulate an on-die laser carrier using an integrated Mach-Zehnder or electro-absorption modulator on a silicon photonics die. Mixing and filtering occur in the optical domain before optical-to-electrical conversion into the memory fabric.

1005. **Zero-Latency Frequency & Gain Sweeping via MMIO Doorbells**  
      Tuning local oscillators (LO), adjusting phase-locked loops (PLLs), or configuring variable-gain amplifiers (VGAs) does not call kernel device drivers or execute slow SPI transactions. An actor writes an atomic 64-bit word to an MMIO register; the hardware synthesizer locks to the new frequency in **under 20 nanoseconds**.

1006. **Polyphase Filter Banks (PFB) with Zero FFT Spectral Leakage**  
      Synthesize an M-channel Polyphase Filter Bank directly in FPGA DSP slices. Incoming wideband spectrum is partitioned into clean, non-overlapping channelized sub-bands with 90 dB stopband rejection before the FFT stage, eliminating spectral leakage and side-lobe contamination in high-density RF environments.

1007. **Sub-Nyquist Compressive Sensing directly in ADC Logic**  
      When monitoring ultra-wide multi-gigahertz bands for sparse RF signals (e.g., electronic warfare or wideband spectrum sensing): ADC logic executes continuous pseudo-random modulation and integration, sampling signals at a fraction of the Shannon-Nyquist rate while reconstructing signal vectors directly in GPU shared memory.

1008. **Sub-Picosecond Phase Alignment Across Multi-Transceiver Arrays**  
      Calibrate phased-array antennas dynamically: on-die Time-to-Digital Converters (TDCs) monitor round-trip electrical delays to individual antenna elements, adjusting programmable digital delay lines in single-cycle logic to maintain phase coherence within **50 femtoseconds** across thousands of active antenna elements.

1009. **Hardware-Enforced Spectral Emission Mask Guardrails**  
      Prevent transmission errors from violating FCC/regulatory frequency allocations. An immutable hardware monitor sits between the digital baseband pipeline and the DAC: if calculated spectral power exceeds authorized out-of-band power spectral density (PSD) limits, the hardware pulls the DAC output enable line low in **under 2 nanoseconds**.

1010. **White Rabbit PTP Timestamp Ingestion at Sample Boundaries**  
      Every ingested I/Q chunk is tagged with an immutable White Rabbit IEEE 1588 PTP timestamp right at the ADC physical converter boundary. Signals received across antennas separated by kilometers can be phase-correlated and synthesized into distributed interferometers with absolute physical causal synchronization.
