<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar LII (52/126); status: DRAFT -->

# Pillar LII: Relational Algebra in Silicon & Hardware Query Engines

**Breakthroughs 501-510** - tag `db` - [index](../index.md)


501. **Direct Compilation of Relational Algebra to GPU SASS & FPGA RTL**  
     Abolish SQL interpreters, Volcano iterator models (`next()` loops), and generic LLVM JIT query compilers. Declarative queries compile ahead-of-time (AOT) into raw GPU machine instructions or pipelined FPGA bitstreams. Selections, projections, and joins become single-pass hardware streaming circuits operating at memory-bus wire speed.

502. **Branchless SIMD Predicate Filtering via `VPCOMPRESSD`**  
     Execute column scans with zero branch misprediction penalties using AVX-512 / AVX10 vector compare and compress instructions:
     ```asm
     vcmppd k1, zmm0, zmm1, 1       // k1 = vector mask of (Column <= Threshold)
     vpcompressd [rdi] {k1}, zmm0   // Write only matching records contiguously to output
     ```
     Filtering billions of rows runs at the physical saturation limit of L3/CXL memory bandwidth.

503. **FPGA-Pipelined Streaming Hash Joins**  
     Build hash join engines directly in FPGA logic. The build table is hashed into high-speed on-chip UltraRAM (URAM) using hardware Murmur3 or cuckoo-hash pipelines. As the probe table streams across the PCIe/CXL bus, matches are emitted on every single 500 MHz clock tick, executing joins with zero memory-materialization overhead.

504. **Processing-in-Memory (PIM) Aggregations in CXL.mem Dies**  
     Push database aggregation operators (`SUM`, `COUNT`, `MIN`, `MAX`, `AVG`) directly into the arithmetic-logic units (ALUs) integrated into the CXL DRAM silicon dies. Multi-terabyte table aggregations execute inside the memory chips themselves, streaming only the final scalar result across the bus.

505. **Zero-Copy In-Place Columnar Layouts (Hardware-Native Apache Arrow)**  
     Data layouts in memory mirror the exact columnar specifications of Apache Arrow without conversion layers. Arrays of offsets, validity bitmasks, and contiguous scalar buffers are mapped to cache-line boundaries. Pointers are handed directly to GPU tensor cores or vectorized CPU routines without transformation.

506. **Direct PCIe TLP Header Packet-Level Filtering**  
     Synthesize an ingress filter in the PCIe root complex or FPGA bridge. When streaming data from NVMe flash to system RAM, the hardware reads row attributes directly from incoming PCIe Transaction Layer Packets (TLPs). Non-qualifying rows are discarded at the bus controller before landing in host DRAM.

507. **Hardware-Accelerated Radix Sorting via Register-Level Permutations**  
     Sort gigabytes of records in real time using a pipelined, multi-pass hardware Radix Sort implemented across GPU vector registers and shared memory. Eliminates comparative branches entirely, delivering constant-time $O(N)$ deterministic sorting bounds.

508. **Branchless Null-Bitmap Evaluation via Single-Cycle Vector Bitwise Math**  
     Represent column nullability as packed 64-bit hardware bitmasks. Query predicates evaluate `IS NOT NULL` conditions concurrently with numerical comparisons using bitwise `KAND` and `KOR` register operations in a single cycle.

509. **Streaming Window Functions in Hardware Shift Registers**  
     Implement SQL window functions (`ROW_NUMBER()`, `RANK()`, `LEAD()`, `LAG()`) directly in FPGA shift-register pipelines. Aggregations over moving row frames execute continuously as data streams through the bus, completely avoiding temporary table allocations.

510. **Hardware-Triggered Structural Integrity Validation**  
     Check foreign key constraints, unique constraints, and domain invariants combinatorially inside the memory controller upon the execution of store instructions. A write that breaches an invariant triggers a synchronous hardware fault before the cache line is committed.
