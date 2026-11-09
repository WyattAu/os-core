<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XXII (22/126); status: DRAFT -->

# Pillar XXII: Wire-Speed Ingress, Cut-Through Parsing & Protocol Engines

**Breakthroughs 201-210** - tag `hft` - [index](../index.md)


201. **Single-Cycle AXI4-Stream SBE / FAST Frame Demux**  
     Implement a hardware parser on the FPGA that processes incoming 512-bit AXI4-Stream words from the 10/25GbE MAC in a single clock cycle. Field extraction (Template ID, Message Length, Sequence Number) is achieved using purely combinatorial bit-slicing logic without buffering the Ethernet frame in a FIFO, achieving cut-through parsing latency under 3.2 ns.

202. **Pre-Computed Speculative OUCH/FIX Framing in TX BRAM**  
     Store fully formed, wire-ready Ethernet, IP, TCP, and exchange-protocol headers (e.g., Nasdaq OUCH 5.0) in high-speed FPGA UltraRAM (URAM). When an execution signal fires, the system only writes the dynamic payload fields:
     ```
     [Quantity: 4 bytes] [Price: 4 bytes] [Side: 1 byte] [ClOrdID: 8 bytes]
     ```
     This triggers the transmitter on the same cycle, cutting transmission framing latency to under 5 ns.

203. **Hardware-Level Cut-Through Order Transmission with Bad-FCS Poisoning**  
     To achieve negative or near-zero latency risk validation: begin streaming the outbound order frame (Preamble, MAC, IP, TCP headers) onto the physical wire *before* the internal alpha signal or pre-trade risk check has completely finished calculating. If the risk engine asserts a violation in the final clock cycle, the FPGA purposely asserts an **Invalid Frame Check Sequence (FCS)** / CRC32 error, forcing the exchange's edge switch to drop the frame at the MAC layer.

204. **Hardware Multicast A/B Feed Deduplication in FPGA Silicon**  
     Exchange feeds publish identical data over two distinct physical links (Feed A and Feed B). Implement a dual-MAC cut-through arbiter in FPGA logic: whichever packet presents its 64-bit sequence number first wins the arbiter; the redundant packet on the alternate channel is dropped mid-stream at the 16th byte, completely eliminating software feed-handler arbitration.

205. **Zero-Copy, Single-Cycle MoldUDP64 Packet Reassembly**  
     Nasdaq’s MoldUDP64 protocol wraps multiple message blocks in a single UDP payload. Build an FPGA pipeline with dynamic header offset pointers: as the frame streams across the AXI bus, each sub-message is split into parallel internal buses and dispatched to distinct order book pipelines simultaneously without memory copying.

206. **Pure-Hardware TCP Stack for Order Execution (Zero-CPU Intermediation)**  
     Implement the full TCP state machine (RFC 793 / RFC 9293) entirely within synthesizable RTL. TCP Sequence and Acknowledgment numbers, window sizing, and session handshakes are computed in hardware registers. Transmitting an order requires no OS socket interaction; an internal signal asserts a strobe that flushes the pre-calculated TCP segment directly out of the PHY transceiver.

207. **Layer-1 Optical Splitter Ingress to Parallel Heterogeneous Pipelines**  
     Direct optical fiber from the exchange cross-connect into a 70/30 or 50/50 passive optical splitter on the network card. The primary optical signal feeds the FPGA for sub-nanosecond top-of-book processing, while the secondary optical split feeds a second transceiver directly mapped into a GPU or CPU for deep-book statistical processing, eliminating all internal PCIe packet replication.

208. **Branchless CME MDP 3.0 Variable-Length Int Extraction via AVX-512**  
     If running fallback logic on the CPU, decode Simple Binary Encoding (SBE) repeating groups using single-instruction vector gathers:
     ```rust
     // Extract 8 non-contiguous 32-bit price levels in one instruction
     let prices = _mm256_i32gather_epi32::<4>(base_ptr, index_vector);
     ```
     This parses entire price ladders without conditional jump instructions.

209. **Hardware-Level Sequence Gap Detection & Auto-Replay Requests**  
     The FPGA tracking the UDP sequence counter detects packet loss instantly when $\text{Seq}_{\text{incoming}} > \text{Seq}_{\text{expected}} + 1$. The FPGA immediately triggers a dedicated TCP client embedded in silicon to format and transmit a TCP historical-replay request to the exchange replay server without notifying the host CPU.

210. **Sub-Nanosecond Line-Rate Bitmask Message Filtering**  
     Before an incoming frame even passes the MAC preamble check, filter out irrelevant stock tickers or instrument IDs via an exact-match bitmask register array:
     ```systemverilog
     wire match = (rx_axis_tdata[127:64] == TARGET_SECURITY_ID);
     if (!match) rx_axis_tready <= 1'b0; // Dump packet immediately
     ```
     Irrelevant updates are discarded before they ever touch downstream FIFO or memory structures.
