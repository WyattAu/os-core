<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XXXV (35/126); status: DRAFT -->

# Pillar XXXV: Light Transport, Photorealistic Physics & Wave Optics

**Breakthroughs 331-340** - tag `gfx` - [index](../index.md)


331. **Continuous Spectral Rendering Across the Electromagnetic Spectrum**  
     Abolish the RGB tri-stimulus rendering model. Represent light as a continuous spectral power distribution (SPD) sampled across 16 discrete wavelengths ($\lambda \in [380, 780] \text{ nm}$) packed into vector registers. Physical phenomena like chromatic dispersion, Rayleigh scattering, and thin-film interference emerge naturally from the physics.

332. **Hardware-Accelerated Stokes Vector Polarization Tracking**  
     Trace light as a 4-element **Stokes Vector** $\mathbf{S} = [I, Q, U, V]^T$, tracking total intensity, linear polarization, and circular polarization. Reflective surfaces evaluate Mueller matrices directly in GPU ALUs, accurately rendering Brewster's angle, glare, and birefringence through anisotropic crystals.

333. **Direct Monte Carlo Solution of the Radiative Transfer Equation (RTE)**  
     Compute volume scattering (clouds, smoke, biological tissue) by solving the full integro-differential RTE:
     $$\omega \cdot \nabla L(\mathbf{x}, \omega) = -(\sigma_a + \sigma_s) L(\mathbf{x}, \omega) + \sigma_a L_e(\mathbf{x}, \omega) + \sigma_s \int_{4\pi} p(\omega', \omega) L(\mathbf{x}, \omega') d\omega'$$
     Execute random-walk null-collision algorithms (delta-tracking) directly inside ray-tracing cores with zero pre-computed voxel grids.

334. **Path Guiding via Real-Time Normalized Haar Wavelet Trees**  
     Incoming radiance distributions are mapped into dynamic spatial-directional tree structures in CXL memory. The path tracer samples light paths by querying real-time Haar wavelet distributions, focusing rays strictly into directions with high light contributions and eliminating wasted rays in dark corridors.

335. **Wave-Optics Diffraction via Wigner Distribution Functions**  
     Simulate microscopic surface wave-optics (diffraction gratings, laser speckle, structural color on bird feathers) by evaluating the Wigner Distribution Function directly on surface interaction shaders, bridging classical geometric optics with quantum wave mechanics.

336. **Direct Integration of Sub-Surface Scattering (BSSRDF) via Diffusion Profiles**  
     Compute subsurface scattering across human skin and marble using directional dipole or random-walk BSSRDF evaluations. The GPU samples internal scattering distance kernels analytically, eliminating separate multi-pass screen-space blur hacks.

337. **Hardware-Accelerated Reservoir-Sampled Spatio-Temporal ReSTIR**  
     Implement Reservoir-Sampled Spatio-Temporal Importance Resampling (ReSTIR) directly in hardware shared memory. Millions of dynamic light sources are sampled in real time by streaming candidates through spatial-temporal reservoirs with near-zero memory footprint.

338. **Microfacet Distribution Functions with Proven Energy Conservation**  
     Surface roughness is evaluated using GGX and Beckmann distributions formulated with rigorous multiple-scattering compensation terms. Formally prove that the Bidirectional Reflectance Distribution Function (BRDF) satisfies energy conservation:
     $$\forall \omega_i, \quad \int_{\Omega} f_r(\omega_i, \omega_o) (\omega_o \cdot \mathbf{n}) d\omega_o \le 1$$
     surfaces cannot artificially emit energy or darken at grazing angles.

339. **Wavefront Path Tracing with Ray Compaction in Scratchpad Memory**  
     Eliminate branch divergence in path tracing. Deconstruct path tracing into discrete stages (Ray Generation, Intersection, Surface Evaluation, Light Sampling). Rays are sorted by hit-material and compacted in-place using single-cycle SIMD ballot instructions, ensuring every GPU compute lane executes identical code.

340. **Direct In-Memory Radiance Caching via Neural Hash Grids**  
     Cache multi-bounce global illumination inside spatial multi-resolution hash tables. An ultra-compact neural network (evaluated in GPU Tensor/Matrix cores in FP8) interpolates indirect radiance across the scene, rendering clean, noise-free global illumination in a single pass.
