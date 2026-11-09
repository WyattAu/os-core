<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar C (100/126); status: DRAFT -->

# Pillar C: Zero-Trust Sandboxing & Hardware Memory Sealing

**Breakthroughs 981-990** - tag `area` - [index](../index.md)


981. **Hardware Memory Sealing via Memory Protection Keys (MPKs)**  
     Partition guest enclaves using x86 PKU (Protection Keys for User Pages) / ARM Memory Tagging Extensions (MTE). The processor assigns a distinct hardware key to every sandboxed enclave; switching enclaves updates the `PKRU` register in a single instruction, rendering the memory of adjacent enclaves physically unreadable at the silicon layer.

982. **Total Elimination of Ambient Syscall Authority (Capability Filtering)**  
     Legacy binaries do not have access to the full POSIX API. Each sandbox is instantiated with an explicit capability bitmask: if a binary is not granted the `Cap_Network_Egress` token, the hardware instruction trap for network-related syscalls fails instantly with a deterministic error code.

983. **Silicon-Level Branch Predictor Partitioning**  
     Neutralize Spectre-v2, Branch Target Injection (BTI), and Return Stack Buffer (RSB) cross-talk. When transitioning execution into a sandboxed enclave, the core’s internal branch predictor state is isolated using hardware tag registers, physically blocking speculative branch injection between legacy and native domains.

984. **Hardware-Enforced Network Egress/Ingress Filtering**  
     Network traffic originating from an untrusted guest passes through an integrated FPGA packet filter directly on the NIC. The hardware validates source IP, destination port, and protocol against a cryptographic capability manifest burned into the NIC’s memory, blocking unauthorized outbound exfiltration at wire speed.

985. **Automatic Landauer-Optimized Memory Wiping upon Enclave Tear-Down**  
     When a sandboxed process terminates, its memory space is not left uninitialized. The physical memory controller executes a high-speed parallel clear, cycling through hardware pre-cleared memory lines to reset physical cell states with minimal thermal entropy generation, ensuring zero data leakage to subsequent enclaves.

986. **Autonomous Hardware Tripwires in Guard Pages**  
     Surround sandboxed memory regions with hardware guard pages mapped to dedicated tripwire addresses. If an untrusted binary suffers a buffer overflow or attempts to probe adjacent memory, the hardware bus arbiter instantly freezes the enclave within a single clock cycle.

987. **Zero-Trust Memory Bus Encryption for Sandboxed Enclaves**  
     All data written to external DRAM or CXL memory by a sandboxed enclave is encrypted in-flight by the memory controller using an ephemeral, hardware-generated AES-256 / ChaCha20 key. Even if an attacker physically taps the memory bus, the guest memory presents as pure cryptographic noise.

988. **Elimination of Root and Setuid Privilege Escalation**  
     The concept of `root` (UID 0) does not exist in the host architecture. If a legacy binary executes `setuid(0)` or attempts a local privilege escalation exploit, the syscall translation shim trivially acknowledges the call with a dummy success code while granting zero additional capability tokens, neutralizing privilege escalation by design.

989. **Hardware-Enforced CPU Execution Quotas via Micro-Op Counters**  
     Prevent untrusted guests from executing algorithmic Denial-of-Service attacks (e.g., infinite loops or crypto-mining). The core’s instruction retirement unit decrements a hardware micro-op counter; when the budget reaches zero, the core enters a low-power quiescent state until refreshed by the host scheduler.

990. **Hardware-Locked Read-Only Binaries and Libraries**  
     Legacy application code and shared libraries are mapped into physical memory marked as strictly immutable. The processor physically forbids write cycles to these pages; self-modifying exploits, in-memory shellcode injection, and ROP gadget spraying are eliminated at the memory-controller level.
