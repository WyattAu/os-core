<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XIX (19/126); status: DRAFT -->

# Pillar XIX: Concrete CPU $\leftrightarrow$ GPU $\leftrightarrow$ FPGA Zero-Copy Wire Protocols

**Breakthroughs 171-180** - tag `K5` - [index](../index.md)


171. **FPGA AXI4-Master to PCIe Bridge for Direct GPU HBM Ingress**  
     Configure an AXI4-to-PCIe IP core on the FPGA. Wire the AXI Master Write channels (`AWADDR`, `WDATA`, `WVALID`) directly to a PCIe Memory Write 64-bit (MWr64) TLP generator targeting the Base Address Register (BAR1) of the GPU. Incoming network packets pass straight from the MAC directly to GPU High-Bandwidth Memory without touching CPU memory.

172. **Direct GPU Doorbell Ringing from FPGA Silicon**  
     The FPGA firmware holds the mapped physical address of the GPU Work-Queue Doorbell register. Upon completing an ingress packet stream, the FPGA generates a single 32-bit PCIe write TLP to the GPU’s doorbell register:
     ```
     TLP: [Format: 32-bit Write, Length: 1 DW, Address: GPU_DOORBELL_PHYS_ADDR, Data: Queue_Tail_Idx]
     ```
     This triggers the GPU hardware warp scheduler to begin compute execution immediately.

173. **CXL.mem HDM (Host-Managed Device Memory) Decoder Setup**  
     Program the CXL Host-Managed Device Memory (HDM) Decoder registers in the device’s PCIe Designated Vendor-Specific Extended Capability (DVSEC) block:
     * `HDM_Decoder_0_Base_High/Low`: Set to the physical address offset assigned to the device.
     * `HDM_Decoder_0_Size_High/Low`: Set to the size of the accelerator's internal memory.
     * `HDM_Decoder_0_Control`: Bit 0 set to 1 (Enable), Bits 3:1 set to `000` (Direct CXL.mem target).  
     This maps the device's internal DRAM/HBM directly into the CPU's coherent physical address space.

174. **Bypassing Linux HMM: Static GPU BAR1 Mapping**  
     Instead of relying on the Linux Heterogeneous Memory Management (HMM) page-fault migration path, configure the GPU to run with a **Resizeable BAR** (up to 64GB/128GB). Map the entire GPU memory physically into the kernel’s unified address space at boot time, making GPU memory instantly accessible via standard CPU pointers:
     ```rust
     let gpu_mem: *mut u32 = 0xF000_0000_0000 as *mut u32; // Direct BAR1 Physical Pointer
     unsafe { *gpu_mem.add(offset) = command; }
     ```

175. **GPUDirect Storage (GDS) via NVMe Direct Memory P2P Mapping**  
     Program an NVMe controller’s Submission Queue Tail Doorbell register (`SQ0TDBL`, offset `0x1000`) with a physical buffer pointer pointing directly into the GPU's BAR1 memory space. When the NVMe DMA engine fetches the read command, it transfers data blocks directly across the PCIe switch into GPU HBM, skipping host RAM.

176. **Zero-Copy Network Header Demux via FPGA Direct Register Slicing**  
     In the FPGA ingress pipeline (written in SystemVerilog/VHDL), implement a single-clock-cycle bit-shift parsing register:
     ```systemverilog
     // L2/L3/L4 Parsing in a single clock cycle
     assign eth_type = rx_axis_tdata[111:96];
     assign ip_proto  = rx_axis_tdata[191:184];
     assign dst_port  = rx_axis_tdata[255:240];
     ```
     Headers are routed to internal FPGA control logic, while the variable-length payload stream is directed to the outbound PCIe DMA FIFO on the same clock cycle.

177. **Hardware Checksum Validation via `CRC32` and `PCLMULQDQ`**  
     Compute protocol and data-frame integrity checks in a single clock cycle using x86 vector carry-less multiplication (`PCLMULQDQ`) and the streaming `CRC32` instruction:
     ```rust
     unsafe {
         crc = core::arch::x86_64::_mm_crc32_u64(crc, payload_chunk);
     }
     ```
     This replaces software-based look-up tables with hardware pipelined execution.

178. **PCIe Direct Memory P2P Access via Dual-Endpoint BAR Exposure**  
     Expose a 4GB BAR space on both an FPGA card and a GPU card attached to the same PCIe downstream switch. Configure the switch’s upstream and downstream routing tables to route TLPs matching target address ranges directly between the two ports, terminating the transactions internally within the switch crossbar without notifying the host CPU root complex.

179. **Direct NVMe Completion Queue Interception by FPGA**  
     Map the NVMe Completion Queue (CQ) physical address directly into an FPGA BRAM range. The NVMe controller posts completion entries straight to the FPGA, allowing the FPGA to trigger the next stage of computation without the CPU ever reading the completion block.

180. **Hardware-Triggered Adaptive Rate Limiting via Credit Accounting**  
     Implement a down-counter on the FPGA connected to the PCIe flow-control credit registers. When PCIe buffer credits for downstream GPU writes dip below a critical threshold, the FPGA ingress MAC directly drops lower-priority network packets at the physical transceiver, preventing upstream buffer bloat.
