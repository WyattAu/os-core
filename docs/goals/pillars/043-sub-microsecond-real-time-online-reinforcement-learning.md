<!-- corpus: verbatim split of goals.md (sha256:6c9cf0a81a3d); pillar XLIII (43/126); status: DRAFT -->

# Pillar XLIII: Sub-Microsecond Real-Time Online Reinforcement Learning

**Breakthroughs 411-420** - tag `ml` - [index](../index.md)


411. **Closed-Loop Silicon Actor-Critic Architecture**  
     Decouple Actor and Critic onto heterogeneous silicon:
     * **Actor (Policy):** Synthesized in FPGA RTL / GPU Tensor cores for deterministic sub-microsecond action selection ($t_{\text{inference}} < 250\text{ ns}$).
     * **Critic (Value Function):** Evaluated concurrently on GPU/CPU to compute temporal-difference (TD) errors and update value surfaces in background epochs.

412. **Lock-Free Hardware Circular Replay Buffer in GPU HBM**  
     Experience tuples $(s_t, a_t, r_t, s_{t+1}, \gamma)$ are written directly to a continuous ring buffer in GPU High-Bandwidth Memory (HBM) using atomic PCIe P2P writes from the inference core. The training pipeline samples minibatches directly from this buffer via strided SIMD gathers with zero host CPU copying.

413. **Single-Step Temporal-Difference (TD) Weight Updates in Scratchpad SRAM**  
     For online tabular or linear-function approximation RL: the TD error update:
     $$\theta_{t+1} = \theta_t + \alpha \delta_t \nabla_\theta V(s_t)$$
     is computed inside on-chip scratchpad SRAM via fixed-point DSP math in a single clock cycle immediately upon observing reward $r_t$, eliminating offline batch training entirely.

414. **Hardware-Enforced Action Masking via Capability Invariants**  
     In high-stakes environments (e.g., financial execution, autonomous control), the RL agent cannot select illegal or catastrophic actions. The action probability vector emitted by the softmax layer is bitwise-ANDed with a hardware-enforced permission mask in a single clock cycle before the action index is decoded:
     ```systemverilog
     assign masked_action_logits = raw_logits & hardware_capability_mask;
     ```

415. **Rolling Hardware Generalized Advantage Estimation (GAE)**  
     Compute GAE($\lambda$) continuously using a dedicated hardware accumulator:
     $$\hat{A}_t = \delta_t + (\gamma \lambda) \hat{A}_{t+1}$$
     Incoming rewards and value estimates stream through a pipelined multiply-accumulate (MAC) unit, computing advantages on-the-fly and eliminating post-trajectory trajectory-scanning passes.

416. **Autonomous Multi-Agent Policy Arbitration in Crossbar Silicon**  
     When multiple RL agents control distinct subsystems (e.g., multiple trading strategies or robotic actuators), their proposed actions are arbitrated in an FPGA crossbar switch using a game-theoretic Nash equilibrium solver synthesized in combinatorial logic, resolving resource conflicts in under 15 ns.

417. **Dynamic Reward Scaling via Running Welford Accumulators in Registers**  
     Normalize non-stationary rewards online without storing historical reward arrays. Implement Welford’s algorithm in vector registers to update running variance $\sigma^2$ and mean $\mu$ incrementally upon every environment transition in 4 clock cycles:
     $$\mu_k = \mu_{k-1} + \frac{x_k - \mu_{k-1}}{k}, \quad M_{2,k} = M_{2,k-1} + (x_k - \mu_{k-1})(x_k - \mu_k)$$

418. **Continuous Entropy-Regularized Exploration via Physical TRNG**  
     Instead of pseudo-random $\epsilon$-greedy exploration, policy action sampling draws continuous stochastic noise directly from on-chip physical thermal noise generators (TRNGs), eliminating deterministic PRNG periodicity and preventing adversaries from predicting agent exploration trajectories.

419. **Real-Time On-Die Model Predictive Control (MPC) Fallback**  
     Pair the neural RL policy with an analytical, linearized quadratic regulator (LQR) or MPC model implemented in FPGA DSPs. If the neural policy's confidence score (entropy) drops below a verified threshold, execution falls back instantly to the analytical LQR controller within 20 nanoseconds.

420. **Asynchronous Parameter Synchronization over CXL.cache**  
     Model weights are updated by the training engine and mirrored to the inference engine over CXL.cache. The inference engine reads weights directly from coherent L3 cache lines with zero execution pausing or mutex locking.
