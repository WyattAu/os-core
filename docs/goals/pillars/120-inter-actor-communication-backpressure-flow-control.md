<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar CXX (120/126); status: DRAFT -->

# Pillar CXX: Inter-Actor Communication, Backpressure & Flow Control

**Breakthroughs 1181-1190** - tag `area` - [index](../index.md)


1181. **Credit-Based Hardware Flow Control on Mailboxes**  
      Prevent actor mailbox overflows in silicon. Senders acquire hardware credit tokens before writing to a recipient actor's mailbox; if the mailbox approaches capacity, the sender’s hardware ring throttles at the bus layer, eliminating queue buffer-overflow drops.

1182. **Direct Register-to-Register Actor Pipes for Co-Located Fibers**  
      When two actors are scheduled on adjacent execution pipelines of the same core, message passing bypasses memory completely: data is passed directly through dedicated register-to-register crossbar links in **zero nanoseconds**.

1183. **Optical Crossbar Inter-Chassis Actor Routing**  
      When an actor sends a message to an actor residing in a different physical server rack, the message is formatted as a 64-byte CXL/optical Flit. The optical crossbar switch routes the flit directly into the remote node's physical mailbox ring in **under 45 nanoseconds**.

1184. **Wait-Free Priority Channels for Urgent Control Descriptors**  
      Actor mailboxes incorporate a dual-tier hardware structure: standard computational messages flow through the circular queue, while urgent control messages (e.g., risk limits, emergency halts) travel over a dedicated priority lane that bypasses the queue head instantly.

1185. **Single-Cycle Hardware Bitmask Pattern Matching**  
      Selective message reception (e.g., matching on specific transaction IDs or message schemas) does not traverse the mailbox linearly in software. The hardware mailbox controller evaluates a combinatorial bitmask comparator across queue slots in a **single clock cycle**, extracting the matching message immediately.

1186. **Ephemeral One-Shot Reply Channels**  
      Request-response interactions utilize linear, single-use reply capabilities. The responding actor consumes the token to return the result; once used, the reply channel physically dissolves, mathematically preventing duplicate responses and stale message routing.

1187. **Single-Bus-Transaction Actor Multicasting**  
      Broadcasting a state update across hundreds of subscribed actors does not execute a software loop. An atomic write to a multicast capability register replicates the message across the memory crossbar to all subscriber mailboxes simultaneously in a single physical memory cycle.

1188. **Actor-Level Hardware Backpressure Signaling to Ingress Transceivers**  
      If downstream consumer actors experience heavy execution backlogs, the mailbox credit deficit propagates directly down to the physical network card or FPGA transceiver, pacing incoming line-rate packet streams at the physical optical layer.

1189. **Dynamic In-Register Mailbox Coalescing for Batch Processing**  
      When an actor wakes to high message density, the hardware mailbox arbiter coalesces up to eight 64-byte message descriptors directly into wide 512-bit vector registers (`ZMM0`–`ZMM3`), allowing vectorized batch processing with zero memory staging overhead.

1190. **Sub-Nanosecond Actor Awakening via Direct Memory Snooping**  
      When a sender writes a descriptor to an idle actor's mailbox, the target core’s instruction fetch unit detects the cache-line snoop event and issues the first instruction of the receiving actor within **under 1.5 nanoseconds**.
