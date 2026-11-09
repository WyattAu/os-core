<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar CXIII (113/126); status: DRAFT -->

# Pillar CXIII: Capability-Gated Dependencies & Principle of Least Authority (POLA)

**Breakthroughs 1111-1120** - tag `area` - [index](../index.md)


1111. **Strict Compile-Time Capability Manifests**  
      Third-party dependencies possess **zero ambient authority**. A math library, JSON parser, or compression algorithm cannot access the network, storage, system clock, or hardware counters unless explicitly granted a capability token in its manifest:
      ```custom_lang
      package FastMatrixMultiplication {
          capabilities: [], // Zero capabilities: pure mathematical compute only
      }
      ```

1112. **Type-Level Capability Enforcement Across Function Boundaries**  
      If a dependency attempts to invoke a hardware capability not declared in its manifest (e.g., a mathematical utility attempting to open an outbound socket), the compiler rejects the code at the type level:
      $$\text{TypeCheckFailure: Dependency } \text{LibA} \text{ lacks capability } \text{Cap\_Network\_Egress}$$

1113. **Hardware Memory Isolation via Dedicated Protection Keys (MPKs)**  
      At runtime, every third-party dependency is assigned an isolated hardware **Memory Protection Key (MPK)** by the processor. When execution crosses a dependency boundary, the CPU updates the `PKRU` register in a single instruction; third-party code is physically blocked by MMU silicon from reading the caller's stack or private memory lines.

1114. **Dynamic Capability Attenuation on Transitive Imports**  
      When Package $A$ imports Package $B$, it can strictly attenuate Package $B$’s permissions:
      ```custom_lang
      import NetworkLogger from "capid:7f9a..." with capabilities: [Cap_Socket_Send("10.0.0.1:443")];
      ```
      Even if `NetworkLogger` requested broad network access, it is physically restricted to a single, hardcoded destination IP and port.

1115. **Total Elimination of File System Scraping**  
      Dependencies cannot query the file system root (`/`), temporary directories (`/tmp`), or parent directories (`../`). File access is replaced by passing explicit, isolated capability handles to specific memory-mapped persistent storage slabs.

1116. **Prohibition of Dynamic Code Evaluation & Unchecked FFI**  
      Third-party dependencies are physically forbidden from utilizing `eval`, dynamic string-to-code compilation, self-modifying assembly, or unchecked Foreign Function Interfaces (FFI). All external calls must be statically declared, strongly typed, and verified during compilation.

1117. **Hardware-Enforced Instruction Traps on Undeclared Probing**  
      If a dependency attempts to execute instructions that probe host machine characteristics (e.g., executing `CPUID`, reading performance counters, or timing memory access to mount cache attacks), the hardware decoder traps instantly, terminating the offending enclave in **under 2 nanoseconds**.

1118. **Sub-Enclave Linear Memory Slicing for Untrusted Code**  
      Allocate third-party dependencies strictly within linear, bounded memory arenas. Out-of-bounds pointer reads or writes do not trigger undefined behavior or silent memory corruption; they hit hardware base-and-bounds range comparators, causing an immediate, isolated thread fault.

1119. **Complete Transitive Capability Footprint Reflection**  
      The operator CLI and IDE inspect the total transitive capability footprint of an entire project as a single, closed Boolean formula. Developers can audit whether *any* sub-dependency in a graph of 5,000 packages possesses authority to transmit network packets or write to non-volatile storage.

1120. **Zero-Latency In-Memory Sandboxing for Plugin Ecosystems**  
      Third-party plugins (e.g., user-submitted trading strategies, analytical filters, UI extensions) run inside the primary application's address space without process boundary context switches, protected entirely by hardware capability tokens and formal Lean 4 confinement proofs.
