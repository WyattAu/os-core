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
ci: contract fmt-check lint test vet lean

repro:
	./scripts/repro-check.sh

bench:
	./scripts/bench-budget.sh

bench-update:
	./scripts/bench-budget.sh --update

clean:
	cargo clean
