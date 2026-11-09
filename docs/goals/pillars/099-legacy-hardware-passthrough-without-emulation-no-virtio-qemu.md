<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XCIX (99/126); status: DRAFT -->

# Pillar XCIX: Legacy Hardware Passthrough without Emulation (No VirtIO / QEMU)

**Breakthroughs 971-980** - tag `area` - [index](../index.md)


971. **Total Abolition of VirtIO Software Driver Shims**  
     Abolish `virtio-net`, `virtio-blk`, and `virtio-scsi`. Legacy guests do not talk to virtualized software device models running in a hypervisor process. Devices are exposed via **native SR-IOV Virtual Functions (VFs)** or synthetic hardware BARs mapped directly into the guest’s memory space.

972. **Hardware Configuration Space Emulation in Persistent SRAM**  
     When an untrusted guest probes the PCIe bus (scanning PCI Configuration Space via ports `0xCF8`/`0xCFC` or Enhanced Configuration Access Mechanism - ECAM), the read hits an isolated, read-only hardware register template in on-chip SRAM. The guest observes a static, hardware-verified device list with zero VM-exit trap latency.

973. **Direct-to-GPU Warp Submission for Sandboxed Compute**  
     Untrusted Linux applications running CUDA, OpenCL, or Vulkan binaries do not pass commands through host driver daemons. The enclave maps a dedicated hardware GPU work-queue doorbell directly. The GPU hardware scheduler enforces spatial and temporal resource boundaries directly in silicon.

974. **Storage Virtualization via Direct NVMe ZNS Zone Mapping**  
     Legacy guest block I/O requests are translated directly into NVMe Zoned Namespace (ZNS) append commands. The guest writes directly to physical storage sectors reserved for its capability token, completely bypassing host storage stack indirection.

975. **Hardware VF Doorbell Reflectors for Sub-Microsecond Network I/O**  
     The network interface card (NIC) exposes hundreds of independent hardware queue pairs. An untrusted legacy binary is assigned a dedicated queue pair; writing to the network is a direct, unprivileged MMIO write to the NIC’s physical doorbell register, delivering sub-microsecond line-rate networking.

976. **Synthetic Console Display Buffering via DisplayPort Overlay Planes**  
     Legacy Linux framebuffer or TTY output writes to a dedicated virtual video memory range. The hardware display controller maps this range directly to an isolated hardware overlay plane, scanning out the legacy interface in real time without software window capture or compositor copies.

977. **Hardware Intercept of Device Configuration Mutations**  
     If a legacy guest attempts to reconfigure physical device registers (e.g., changing network MAC addresses, altering PCIe link speeds, or reprogramming IOMMU tables), the memory controller silently drops the write and asserts a hardware capability violation trap, preventing device hijacking.

978. **Zero-Memory-Copy Sound Emulation via Direct Audio Ring Slicing**  
     Legacy audio calls (ALSA / OSS interfaces) map directly to a sub-slice of the hardware audio crossbar’s DMA ring. Sound buffers stream directly to the physical DAC with zero format conversion or software mixing overhead.

979. **Autonomous Clock-Gating for Idle Sandboxed Hardware Queues**  
     If a sandboxed legacy guest becomes idle and ceases to ring its device doorbells, the peripheral controller automatically gates power to the assigned Virtual Function registers, preserving energy without requiring software hypervisor power-management daemons.

980. **Deterministic Bus Rate Limiting via Silicon PCIe Credit Pacing**  
     Prevent untrusted guest workloads from saturating the PCIe/CXL bus. The root complex enforces strict hardware credit rate-limits on every assigned Virtual Function, guaranteeing that a malfunctioning or malicious legacy binary cannot starve real-time native tasks.
