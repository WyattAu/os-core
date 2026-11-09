<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar LXXXIII (83/126); status: DRAFT -->

# Pillar LXXXIII: Eradication of Dynamic Linkers & Runtime Indirection (No `ld.so`, No GOT/PLT)

**Breakthroughs 811-820** - tag `area` - [index](../index.md)


811. **Total Abolition of the Global Offset Table (GOT) and Procedure Linkage Table (PLT)**  
     Dynamic libraries historically resolve external functions via GOT/PLT indirect jumps (`jmp *GOT[rip]`), introducing a minimum of one extra memory dereference and pipeline branch stall per function call. In this OS, inter-module calls are resolved at module registration to **direct, static near/far jumps or capability-authenticated calls**, eliminating GOT/PLT indirection entirely.

812. **Elimination of the Dynamic Linker (`ld.so`)**  
     Abolish the thousands of lines of C code running in user space that parse symbols, resolve dependencies, and process relocations before `main()` is invoked. Module linking is executed by the kernel's capability arbiter as an instantaneous, atomic pointer swing in persistent CXL memory.

813. **Sub-Nanosecond Dynamic Function Swapping via Atomic Pointer Redirection**  
     Replacing or upgrading an active library function (hot-patching) does not require restarting processes or rebinding symbols. The caller jumps to a direct capability entry point; swapping the implementation requires a single atomic 64-bit pointer write to the module's export capability register.

814. **Link-Time Elimination of Unused Capability Tokens**  
     If a program imports a wide module (e.g., the complete network stack) but only calls an isolated hashing function, the linker's formal reachability analysis proves the remainder of the module is unreferenced. The linker strips the unreferenced code and permanently drops the unused hardware capabilities (e.g., raw PCIe BAR access) from the application's manifest.

815. **Zero-Cost Cross-Silicon FFI (Foreign Function Interfaces)**  
     Calling an FPGA hardware accelerator or a GPU compute kernel from the CPU uses the exact same syntax and calling convention as a standard internal function call. The linker resolves the callee to an MMIO doorbell instruction sequence or a direct register exchange, eliminating runtime driver abstraction layers.

816. **Elimination of Symbol String Tables at Link Time**  
     Symbols do not exist as ASCII/UTF-8 strings (`printf`, `malloc`) in object files. Symbols are **128-bit cryptographic type-and-name hashes**. The linker resolves dependencies via constant-time integer comparisons or direct hash-table indexing, slashing link times from seconds to microseconds.

817. **Whole-System Dead-Code Elimination Across Component Boundaries**  
     Because all libraries and modules reside in the Single Address Space and are represented as Merkle-DAGs, the linker performs global, whole-system reachability analysis. Any function, register path, or hardware capability that is not transitively reachable from active system entry points is pruned from memory.

818. **AOT Resolved Thread-Local Storage (TLS) Offsets**  
     Abolish general-dynamic and local-dynamic TLS models with their expensive calls to `__tls_get_addr`. Thread-local storage offsets are assigned static, constant offsets relative to the dedicated thread-pointer register (`FS` or `TP`) at compile/link time, reducing all TLS reads to a single-cycle segment-relative memory access.

819. **Deterministic Link-Order Invariance**  
     In traditional toolchains, the order in which libraries are passed to the linker (`-la -lb` vs. `-lb -la`) can alter binary output or cause undefined symbol errors. The clean-slate linker treats dependencies as an unordered, commutative set of capability requirements; link order has mathematically zero effect on the generated binary.

820. **Hardware-Enforced Call-Graph Integrity**  
     The linker outputs a static, hardware-enforced Call Graph Matrix. The CPU branch monitor ensures that function $A$ can only call function $B$ if the transition is explicitly permitted by the linker's compiled adjacency matrix, permanently eliminating forward-edge control-flow hijacking.
