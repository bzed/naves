# SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
# SPDX-License-Identifier: AGPL-3.0-or-later

COVERAGE_MIN = 85

# Temporary and build folders never go under $HOME.
export TMPDIR ?= /tmp
export CARGO_TARGET_DIR ?= /var/tmp/naves-target

CARGO = cargo
LOCKED = --locked
# cargo-fuzz binaries from prebuilt releases default to a musl target, where sanitizers do not work.
FUZZ_HOST = $(shell rustc +nightly -vV | sed -n "s/^host: //p")

.PHONY: fmt fmt-check lint doc reuse deny ca-bundle test fuzz cover coverage-check docs miri deb check

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
	    mkdir -p $(TMPDIR)/naves-fuzz/$$t; \
	    $(CARGO) +nightly fuzz run --target $(FUZZ_HOST) $$t $(TMPDIR)/naves-fuzz/$$t corpus/$$t -- -max_total_time=10 || exit 1; \
	  done; \
	fi

cover:
	$(CARGO) llvm-cov --workspace $(LOCKED) --lcov --output-path lcov.info

coverage-check:
	$(CARGO) llvm-cov --workspace $(LOCKED) --exclude naves-ui --fail-under-lines $(COVERAGE_MIN)

docs:
	mdbook build docs/manual
	lychee --offline --no-progress --root-dir $(CURDIR)/docs/manual/book docs/manual/book

# The CA list that the HTTP client trusts (webpki-roots, from Mozilla) must be the newest release: a CA can be
# revoked, and a build that ships an old list keeps trusting it. Needs network access.
ca-bundle:
	@locked=$$(awk '/^name = "webpki-roots"$$/ {getline; gsub(/"/, "", $$3); print $$3}' Cargo.lock | sort -V | tail -1); \
	latest=$$(curl -fsS -A "naves-ci (https://github.com/bzed/naves)" https://crates.io/api/v1/crates/webpki-roots \
	  | python3 -c 'import json, sys; print(json.load(sys.stdin)["crate"]["max_stable_version"])'); \
	[ -n "$$locked" ] && [ -n "$$latest" ] || { echo "cannot find the webpki-roots version"; exit 1; }; \
	if [ "$$locked" = "$$latest" ]; then echo "webpki-roots $$locked is the newest release"; \
	else echo "webpki-roots $$locked in Cargo.lock, the newest is $$latest: run 'cargo update -p webpki-roots'"; exit 1; fi

miri:
	$(CARGO) +nightly miri test --workspace $(LOCKED)

deb:
	cargo vendor vendor >/dev/null
	dpkg-buildpackage -us -uc -b
	lintian --fail-on error,warning ../naves_*.changes

check: fmt-check lint doc reuse deny test fuzz coverage-check docs
