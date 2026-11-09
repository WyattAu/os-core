<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XXXII (32/126); status: DRAFT -->

# Pillar XXXII: Direct Silicon Display Engines & Zero-Copy Scanout

**Breakthroughs 301-310** - tag `gfx` - [index](../index.md)


301. **Compositor-Free Direct Scanout via Hardware Overlay Engines**  
     Abolish software window compositors (X11, Wayland, Desktop Window Manager). The physical display controller directly scans out multiple independent memory slices using hardware display planes and z-order registers. Windows are hardware viewports; pixel compositing happens inside the display controller’s line buffers on-the-fly at line rate.

302. **Immediate-Mode Just-In-Time (JIT) Scanline Rendering**  
     Eliminate the entire concept of full-screen framebuffers. Using deterministic compute pipelines, ray-march or rasterize pixels *directly ahead of the display’s physical raster beam*. Rendering occurs within a rolling buffer of 16 scanlines, reducing display latency to sub-microsecond intervals.

303. **Direct HBM/CXL-to-DisplayPort PHY Routing**  
     Bypass the PCIe bus for display output. The DisplayPort 2.1 Main Link physical layer (PHY) is wired directly to the CXL/HBM memory controller. The display engine reads pixels directly out of unified memory via 128b/132b encoded Ultra-High Bit Rate (UHBR20) links without software display-driver mediation.

304. **FPGA-Synthesized Native DisplayPort 2.1 / HDMI 2.1 FRL Engines**  
     Synthesize the entire display timing controller, Fixed Rate Link (FRL) packetizer, and High-Definition Content Protection (HDCP) state machine directly within FPGA fabric. Pixels flow from FPGA logic directly into high-speed transceivers (GTX/GTY), completely bypassing GPU display pipelines.

305. **True Zero-Buffer Variable Refresh Rate (VRR / FreeSync / G-Sync)**  
     Traditional VRR extends the vertical front porch to match frame times. This architecture drives the display panel dynamically: when the final compute block signals completion via an MMIO doorbell, the display engine fires the Vertical Sync pulse immediately, displaying the result on the panel in the exact nanosecond calculation completes.

306. **Direct Scanline Color Space Transformation via FPGA LUTs**  
     Color transformations (Linear Rec.2020 $\to$ PQ/HDR10 or ACEScg $\to$ DCI-P3) are applied strictly inside hardware 3D-LUTs embedded directly within the physical display PHY pipeline. Shaders render strictly in physical linear spectral radiance; gamma and tone mapping never cost a single GPU clock cycle.

307. **Hardware Sub-Pixel Precision Scanout with Eye-Tracking Saccade Prediction**  
     The display scanout engine dynamically shifts viewport offsets based on high-frequency (1000 Hz) eye-tracking sensors read over an SPI/PCIe bus. The display controller warps and shifts scanlines on-the-fly, canceling out human saccadic motion latency.

308. **Lossless Framebuffer Memory Slicing via Hardware Block Compression**  
     Display memory is mapped directly using hardware-level tile compression formats (similar to AMD DCC or NVIDIA CBC) exposed directly to the type system. Decompression is carried out in silicon at the display PHY, saving 60–80% of memory-bus bandwidth.

309. **Hardware-Triggered Asynchronous Time-Warp (ATW) Interruption**  
     If a heavy path-tracing frame misses its scanline budget, the display engine automatically triggers an ultra-high-priority compute warp. It reads the previous frame's depth buffer, applies an affine head-pose matrix transform, and outputs a reprojected scanline without a single dropped display frame.

310. **Tear-Free Direct Scanout without V-Sync Locking**  
     Eliminate tearing and eliminate V-Sync wait loops simultaneously: the rasterizer is synchronized to the display controller’s exact physical beam-counter register. The scheduler guarantees that rasterization writes to a given memory address strictly *behind* the beam reader, maintaining single-buffered rendering without tearing.
