# Thin wrapper over scripts/ and cargo — the same verbs everywhere. CI runs
# these same commands via the estate reusable workflow (tier a).

.PHONY: bench bench-update repro build test lint fmt fmt-check coverage mutants vet semver docs contract ci clean

build:
	cargo build --workspace --all-features --locked

test:
	cargo nextest run --workspace --all-features --locked

lint:
	# --all-targets matches the CI quality job (rust-kit clippy denies
	# unwrap_used etc. in test targets too, not just the lib surface).
	cargo clippy --workspace --all-features --all-targets --locked -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

coverage:
	./scripts/coverage.sh

mutants:
	./scripts/mutants.sh

vet:
	cargo vet check --locked

lean:
	./scripts/check-lean.sh

semver:
	./scripts/semver-check.sh

docs:
	cargo doc --workspace --no-deps --all-features

contract:
	./scripts/check-contract.sh

## What CI gates before merge (mirror of .github/workflows/ci.yml):
## `kernel` mirrors the workflow's QEMU job (build + boot + markers).
ci: contract fmt-check lint test vet lean kernel

kernel:
	./scripts/qemu-omniosk.sh
	./scripts/qemu-ota.sh
	./scripts/qemu-limine.sh

repro:
	./scripts/repro-check.sh

# LOCAL leg (not in ci): Kani model-check over the ring cursor algebra
# (ADR-0014 rung C+). Runners don't ship kani.
kani:
	./scripts/check-kani.sh

bench:
	./scripts/bench-budget.sh

bench-update:
	./scripts/bench-budget.sh --update

clean:
	cargo clean
