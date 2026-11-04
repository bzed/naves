# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later

COVERAGE_MIN = 85

# Temporary and build folders never go under $HOME.
export TMPDIR ?= /tmp
export CARGO_TARGET_DIR ?= /tmp/naves-target

CARGO = cargo
LOCKED = --locked

.PHONY: fmt fmt-check lint doc reuse deny test fuzz cover coverage-check docs miri deb check

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

lint:
	$(CARGO) clippy --workspace --all-targets $(LOCKED) -- -D warnings

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps $(LOCKED)

reuse:
	reuse lint

deny:
	$(CARGO) deny check

test:
	$(CARGO) test --workspace --all-targets $(LOCKED)
	$(CARGO) test --workspace --doc $(LOCKED)

fuzz:
	@if [ -d fuzz ]; then \
	  cd fuzz && for t in $$($(CARGO) +nightly fuzz list 2>/dev/null); do \
	    $(CARGO) +nightly fuzz run $$t -- -max_total_time=10 || exit 1; \
	  done; \
	fi

cover:
	$(CARGO) llvm-cov --workspace $(LOCKED) --lcov --output-path lcov.info

coverage-check:
	$(CARGO) llvm-cov --workspace $(LOCKED) --fail-under-lines $(COVERAGE_MIN)

docs:
	mdbook build docs/manual
	lychee --offline --no-progress --root-dir $(CURDIR)/docs/manual/book docs/manual/book

miri:
	$(CARGO) +nightly miri test --workspace $(LOCKED)

deb:
	cargo vendor vendor >/dev/null
	dpkg-buildpackage -us -uc -b
	lintian --fail-on error,warning ../naves_*.changes

check: fmt-check lint doc reuse deny test fuzz coverage-check docs
