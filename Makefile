GRADLE := env -u GRADLE_HOME ./gradlew
CLIENT := client
APK := $(CLIENT)/androidApp/build/outputs/apk/release/androidApp-release.apk
AAB := $(CLIENT)/androidApp/build/outputs/bundle/release/androidApp-release.aab
# Beside the repo rather than hidden in $HOME, because this file is the one thing here that cannot
# be regenerated: lose it and no device that has installed Kampr can ever be updated again.
KEYSTORE := $(CURDIR)/../kampr-android-keys/kampr-release.jks

# `make check` is the gate, and `.github/workflows/ci.yml` runs these same targets, so the two
# cannot drift. `make test-fast` is the inner loop: everything but the tests that need a real
# herdr (see `.config/nextest.toml`), in about twenty seconds.
.PHONY: check check-rust check-client check-packaging fmt lint test test-fast check-darwin \
  ci-live-client ci-bundle-guard

check: check-rust check-client check-packaging

check-rust: fmt lint test

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

test-fast:
	cargo nextest run --workspace

test:
	cargo nextest run --workspace --profile full

check-client:
	scripts/client-check.sh

check-packaging:
	@command -v shellcheck >/dev/null || { echo "check-packaging needs shellcheck on PATH"; exit 1; }
	for f in packaging/*.sh; do sh -n "$$f"; done
	shellcheck --shell=sh packaging/*.sh
	shellcheck scripts/*.sh scripts/probe .claude/hooks/*.sh
	sh packaging/selftest.sh
	sh packaging/kamprctl-selftest.sh
	python3 scripts/check-plugin-manifest.py

# The release builds two Apple targets, and a Linux-only libc constant once compiled clean here
# and failed both of them with the tag already pushed. CI runs this on macOS; on Linux it proves
# nothing `lint` has not.
check-darwin:
	cargo check --workspace --all-targets

# CI only. Each starts from a state a workstation is not in — a fresh runner's empty dist/, a
# herdr server in the default session — and says why in its script.
ci-live-client:
	cargo build -p kampr-cli
	scripts/live-client-check.sh

ci-bundle-guard:
	scripts/bundle-guard-check.sh

.PHONY: android-release android-bundle android-install android-test android-publish android-attach android-clean android-keystore

android-release:
	cd $(CLIENT) && $(GRADLE) :androidApp:assembleRelease
	apksigner verify --verbose $(APK)
	@echo
	@unzip -l $(APK) | grep -c 'composeResources/.*/font/.*\.ttf' | xargs -I{} echo "fonts packaged: {}"
	@ls -l $(APK)

android-bundle:
	cd $(CLIENT) && $(GRADLE) :androidApp:bundleRelease
	@ls -l $(AAB)

android-install: android-release
	adb install -r $(APK)

# KAMPR_NODE=http://10.0.2.2:8793 additionally proves the app can still reach a plain-http node
# on a private address, which is what targetSdk 37 gates behind ACCESS_LOCAL_NETWORK.
android-test:
	cd $(CLIENT) && $(GRADLE) :androidApp:connectedAndroidTest \
	  $(if $(KAMPR_NODE),-Pandroid.testInstrumentationRunnerArguments.kamprNode=$(KAMPR_NODE))

android-publish:
	cd $(CLIENT) && $(GRADLE) :androidApp:publishToKobup

# The release workflow cannot sign an APK (the key is on this machine), so the tag's release gets
# the one android-publish built — refused unless it is the version the tag names.
android-attach:
	@set -e; v=$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1); \
	built=$$(aapt2 dump badging $(APK) | sed -n "s/.*versionName='\([^+']*\).*/\1/p"); \
	if [ "$$built" != "$$v" ]; then echo "the APK is $$built, Cargo.toml is $$v"; exit 1; fi; \
	out=$$(mktemp -d)/kampr-v$$v-android.apk; \
	cp $(APK) $$out; \
	gh release upload v$$v $$out; \
	gh release view v$$v --json assets --jq '[.assets[].name]'

android-clean:
	cd $(CLIENT) && $(GRADLE) :androidApp:clean

android-keystore:
	@test ! -f $(KEYSTORE) || { echo "$(KEYSTORE) already exists — refusing to overwrite."; echo "Overwriting it would orphan every device that already installed Kampr."; exit 1; }
	@mkdir -p $(dir $(KEYSTORE)) && chmod 700 $(dir $(KEYSTORE))
	@PW=$$(head -c 24 /dev/urandom | base64 | tr -d '/+=' | head -c 28); \
	keytool -genkeypair -keystore $(KEYSTORE) -storetype PKCS12 -alias kampr \
	  -keyalg RSA -keysize 4096 -validity 10950 \
	  -dname "CN=Kampr, OU=Kampr, O=oldug, L=, ST=, C=GB" \
	  -storepass "$$PW" -keypass "$$PW"; \
	chmod 600 $(KEYSTORE); \
	touch $(HOME)/.gradle/gradle.properties && chmod 600 $(HOME)/.gradle/gradle.properties; \
	printf '\n# Kampr release signing — keystore lives outside every repo.\nkamprReleaseStoreFile=%s\nkamprReleaseStorePassword=%s\nkamprReleaseKeyAlias=kampr\nkamprReleaseKeyPassword=%s\n' \
	  "$(KEYSTORE)" "$$PW" "$$PW" >> $(HOME)/.gradle/gradle.properties; \
	echo; echo "Back up BOTH of these, off this machine:"; \
	echo "  $(KEYSTORE)"; \
	echo "  the password: $$PW"; \
	echo; \
	echo "Lose either and no device that has installed Kampr can ever be updated again —"; \
	echo "not by kobup, not by hand. The only recovery is uninstall-and-reinstall everywhere."
