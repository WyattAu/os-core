<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar CXVII (117/126); status: DRAFT -->

# Pillar CXVII: Hardware-Backed Silicon Actor Model & Execution Lifecycle

**Breakthroughs 1151-1160** - tag `area` - [index](../index.md)


1151. **Silicon Actor Control Blocks (SACBs) in Persistent SRAM**  
      Abolish heavy process control blocks (PCBs) and thread structures. An actor’s state is stored in an ultra-compact, 64-byte, cache-line-aligned **Silicon Actor Control Block (SACB)** resident in persistent on-chip SRAM/CXL memory:
      ```rust
      #[repr(C, align(64))]
      pub struct SiliconActorControlBlock {
          pub instruction_pointer: u64,
          pub stack_or_scratchpad_base: u64,
          pub mailbox_head: AtomicU32,
          pub mailbox_tail: AtomicU32,
          pub supervisor_id: u32,
          pub capability_mask: u64,
          pub execution_state: u16, // Active, Quiescent, Faulted, Reconstructing
          pub restart_counter: u16,
      }
      ```

1152. **Sub-10-Nanosecond Actor Instantiation**  
      Spawning an actor does not allocate file descriptor tables, copy page directories, or configure kernel namespaces. Instantiation is a single atomic hardware bitmask operation finding a free SACB slot in persistent memory, claiming an execution arena in **under 8 nanoseconds**.

1153. **Universal Heterogeneous Actor Representation**  
      An actor is completely agnostic of the physical silicon executing it. A Silicon Actor can be scheduled interchangeably onto a **CPU vector core, a GPU compute warp, an FPGA streaming pipeline, or a CXL Processing-in-Memory (PIM) die**, communicating through identical mailbox primitives.

1154. **Single-Cycle Context Switching via Shadow Register Bank Swapping**  
      Context switching between actors on a CPU or accelerator core does not push and pop registers to the stack. Dedicated on-die **Shadow Register Banks** swap the active operational registers in a **single clock cycle**, completely eliminating register-spill memory bandwidth overhead.

1155. **Hardware Cache-Line Mailbox Rings Polled via `UMWAIT`**  
      Each actor’s ingress mailbox is an aligned Single-Producer Single-Consumer (SPSC) or Multi-Producer Single-Consumer (MPSC) circular cache-line ring. The receiving core polls its head pointer via `UMONITOR`/`UMWAIT`, dropping into sub-nanosecond low-power sleep and waking instantly upon hardware cache-line snoop detection.

1156. **Zero-Allocation, Zero-Copy Message Handoffs**  
      Messages passed between actors are not serialized or copied across memory boundaries. Messages are passed as **unforgeable, linear capability tokens** pointing directly to immutable physical memory slices. The receiving actor assumes direct ownership; the sending actor's capability is atomically revoked.

1157. **Hardware-Enforced Ingress Message Schema Validation**  
      Before an incoming message descriptor lands in an actor's mailbox, an on-chip hardware comparator verifies that the message’s 32-bit Type Tag and Size conform to the receiving actor's compiled interface contract in **zero clock cycles**, blocking malformed inputs at the silicon boundary.

1158. **Sub-50-Nanosecond Dynamic Actor Silicon Migration**  
      Migrating an active actor (e.g., moving an algorithm from CPU to GPU or vice-versa based on workload density) is achieved by transferring its SACB descriptor across internal memory crossbars in **under 50 nanoseconds**, resuming execution seamlessly on the target silicon.

1159. **Single-Instruction Quiescence via Hardware Power-Down Hints**  
      When an actor exhausts its mailbox, it executes a single machine instruction that sets its SACB state to `Quiescent` and puts its physical execution core into an ultra-low-power retention state, leaving memory lines warm and ready for instant wake-up.

1160. **Self-Reclaiming Ephemeral Actors with Zero Leakage**  
      Short-lived ephemeral actors (spawned to handle a single incoming network packet or transaction) carry linear capability lifecycles: upon executing their final instruction, the SACB allocation bit is cleared, memory is recycled, and the execution slot is freed instantly with **zero garbage collection**.
