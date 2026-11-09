<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XVIII (18/126); status: DRAFT -->

# Pillar XVIII: Concrete Memory Allocators & Data Structure Layouts

**Breakthroughs 161-170** - tag `K1` - [index](../index.md)


161. **Cache-Aligned, Power-of-Two Lock-Free SPSC Ring Layout**  
     Define submission rings with indices explicitly isolated onto distinct 64-byte cache lines:
     ```rust
     #[repr(C, align(64))]
     pub struct SpscRing<T, const N: usize> {
         // Producer cache line
         pub head: core::sync::atomic::AtomicU64,
         _pad0: [u8; 56],
         // Consumer cache line
         pub tail: core::sync::atomic::AtomicU64,
         _pad1: [u8; 56],
         // Data storage: aligned to cache lines, size N must be 2^k
         pub buffer: [T; N],
     }
     ```

162. **Branchless Ring Index Masking**  
     Enforce ring capacity $N = 2^k$. Replace the modulus operation (`% N`) with bitwise AND (`& (N - 1)`):
     ```rust
     let index = head.load(core::sync::atomic::Ordering::Relaxed) & (N - 1);
     ```
     This maps to a single-cycle `and` instruction, completely eliminating division logic and branch misprediction stalls.

163. **64-Byte Cryptographic Capability Token Layout**  
     Structure memory capabilities to match a single x86/ARM cache line:
     ```rust
     #[repr(C, align(64))]
     pub struct HardwareCapability {
         pub base_phys_addr: u64, // Bits 0-63: Physical base
         pub bound_bytes: u64,    // Bits 0-63: Limit bound
         pub permissions: u32,    // Read, Write, Execute, DMA, CXL
         pub pasid: u32,          // Hardware PASID identifier
         pub epoch: u64,          // Allocation generation counter
         pub auth_mac: [u8; 32],  // 256-bit SipHash/HMAC verifying integrity
     }
     ```

164. **Two-Level Direct 1GB Hugepage Physical Mapping**  
     Bypass the standard 4-level/5-level page table walk (`PML4` $\to$ `PDPT` $\to$ `PD` $\to$ `PT`). Configure the PML4 to map straight into a Page Directory Pointer Table (PDPT) with bit 7 (Page Size, `PS`) set to 1. The MMU resolves physical addresses in just **two memory lookups**, caching 1GB regions in a single TLB entry.

165. **Constant-Time O(1) Bitmask Allocator via `TZCNT`**  
     Represent physical frame allocations as a 64-bit word bitmap. Frame allocation reduces to finding the first trailing zero using native assembly:
     ```rust
     let free_index: u64;
     unsafe {
         core::arch::asm!(
             "tzcnt {out}, {mask}",
             "bts {mask}, {out}",
             mask = inout(reg) self.bitmap,
             out = out(reg) free_index
         );
     }
     ```
     This completes in a deterministic 3 cycles on modern architectures.

166. **Direct Physical Address Memory Arenas for IOMMU Pre-Registration**  
     Carve out contiguous, physical, unpaged memory regions at system initialization. Set the IOMMU translation tables to an identity map ($V_{\text{addr}} = P_{\text{addr}}$) across these arenas, completely eliminating the overhead of pinning, unpinning, and virtual-to-physical address translation during DMA.

167. **Hardware-Enforced RCU via Monotonic Epoch Counters**  
     Track reclamation epochs using a single, globally snooped cache line containing an atomic 64-bit counter:
     ```rust
     pub struct GlobalEpoch {
         pub current_epoch: core::sync::atomic::AtomicU64,
     }
     ```
     Compute cores signal their active state by copying the global counter into a core-local register. When all core-local counters advance past epoch $E$, memory associated with epoch $E - 1$ is freed instantly without pauses.

168. **Static Non-Temporal Bulk-Streaming Buffers (`MOVNTI`/`MOVNTDQ`)**  
     When transferring non-operational data (e.g., logging or network packet captures to persistent storage), use streaming non-temporal store instructions:
     ```rust
     unsafe {
         core::arch::x86_64::_mm_stream_si64(target_ptr as *mut i64, value);
     }
     ```
     This writes directly to DRAM/CXL storage write-combining buffers, bypassing the L1/L2/L3 caches and keeping operational cache lines unpolluted.

169. **Hardware L3 Cache Allocation via `IA32_L3_MASK_n`**  
     Partition L3 cache ways using Intel Cache Allocation Technology (CAT). Program the Capacity Bitmask (CBM) in MSR `0xC90` through `0xC97`:
     ```
     MSR 0xC90 (Class of Service 0): 0b1111_0000_0000 (Reserved for Compute Cores)
     MSR 0xC91 (Class of Service 1): 0b0000_1111_1111 (Dedicated exclusively to I/O Queues)
     ```
     Hardware physically prevents compute threads from evicting critical I/O ring descriptors.

170. **Hardware Register Bank Swapping for Fiber Context Switches**  
     Implement sub-5ns fiber/actor switching: allocate dedicated general-purpose registers (`R8`–`R15`) to hold the primary execution context. Context switching reduces to writing the active instruction pointer (`RIP`) into an indirect jump, bypassing stack push/pop sequences entirely.
