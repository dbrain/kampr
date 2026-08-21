GRADLE := env -u GRADLE_HOME ./gradlew
CLIENT := client
APK := $(CLIENT)/androidApp/build/outputs/apk/release/androidApp-release.apk
AAB := $(CLIENT)/androidApp/build/outputs/bundle/release/androidApp-release.aab
# Beside the repo rather than hidden in $HOME, because this file is the one thing here that cannot
# be regenerated: lose it and no device that has installed Kampr can ever be updated again.
KEYSTORE := $(CURDIR)/../kampr-android-keys/kampr-release.jks

.PHONY: android-release android-bundle android-install android-test android-publish android-clean android-keystore

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
