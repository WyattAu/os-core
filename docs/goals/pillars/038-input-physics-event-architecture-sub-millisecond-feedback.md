<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XXXVIII (38/126); status: DRAFT -->

# Pillar XXXVIII: Input Physics, Event Architecture & Sub-Millisecond Feedback

**Breakthroughs 361-370** - tag `ui` - [index](../index.md)


361. **Sub-1-Millisecond Click-to-Photon Hardware Pipeline**  
     By eliminating input event queues, compositor frame latencies, and double buffering, the system achieves an end-to-end latency of **< 900 microseconds** from the mechanical closure of a mouse switch to the emission of updated photons from the display panel.

362. **Zero-Latency Hardware Cursor Plane Injection**  
     The mouse pointer does not participate in the window rendering loop. The pointer’s $(x, y)$ coordinate is written directly to a dedicated silicon hardware overlay plane inside the display controller. The cursor updates synchronously with the physical scanout beam, guaranteeing zero cursor lag under any system load.

363. **Physical-Layer Microsecond Hardware Switch Debouncing in FPGA**  
     Mechanical mouse switches and keys produce noisy electrical oscillations (chatter) during the first 1–5 milliseconds of actuation. Implement an adaptive, continuous digital integrator on the FPGA USB/SPI PHY that confirms mechanical closure in under **2 microseconds**, cutting milliseconds off consumer-grade input latency.

364. **Direct Polled Event Rings (No Interrupt-Driven Input Spikes)**  
     Abolish asynchronous input interrupts (`IRQ 1`, `IRQ 12`). High-precision input devices (mice running at 8000 Hz polling rates, custom trading keyboards) write raw 64-byte event packets directly into a dedicated lock-free Single-Producer Single-Consumer (SPSC) ring buffer mapped straight into the active window’s address space.

365. **Continuous Kinematic Motion Prediction via Kalman Filtering**  
     When polling input at high frequencies, the UI engine runs an unrolled, fixed-point Kalman filter in hardware to predict user hand trajectories $1–2\text{ ms}$ into the future, synchronizing pointer position precisely with the instantaneous phase of the display’s vertical refresh.

366. **Direct Tactile-Visual Phase-Locked Haptics**  
     Integrate linear resonant actuators (LRAs) or piezoelectric tactile actuators driven directly by the UI event pipeline. The tactile pulse is fired in phase with the visual state change down to the microsecond, providing physical tactile feedback matching pixel transitions.

367. **Hardware-Triggered Input Gesture Engines in Silicon**  
     Multi-touch, pinch-to-zoom, and inertial scrolling calculations are executed inside an FPGA or dedicated micro-engine. Raw capacitive touch matrices are converted into affine spatial transforms directly on the ingress PCIe/SPI bus, delivering instant UI transformations without host CPU overhead.

368. **Analog Velocity-Sensitive Keyboard Interaction**  
     For Hall-effect or optical analog keyboards: keys are not discrete binary states (Up/Down). The UI engine ingests a continuous 16-bit analog displacement curve per key, allowing users to modulate scrolling speeds, brush sizes, or trade order quantities through variable physical key pressure.

369. **Audio-Tactile Confirmation Loops via Real-Time DSP Audio Engines**  
     Audio feedback for UI actions (e.g., clicks, alerts, completions) does not route through high-latency desktop audio servers (PulseAudio/PipeWire). Audio buffers are synthesized directly in FPGA/DSP registers and fired directly out of the I2S/HDA hardware audio DAC synchronously with the input event (latency < 2ms).

370. **Deterministic Eye-Gaze Intent Prefetching**  
     Connect eye-tracking hardware directly to the UI capability engine. When the user’s foveal gaze dwells on a window or button for $> 15\text{ ms}$, the system immediately pre-fetches the application’s memory pages into the CPU L3 cache and warms up GPU pipeline registers before the physical mouse click occurs.
