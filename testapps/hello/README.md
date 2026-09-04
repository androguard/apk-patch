# hello — minimal APK for apk-patch smoke tests

Build:

```bash
./scripts/build-hello.sh
# → testapps/hello/build/hello.apk
```

Roundtrip / inject:

```bash
./scripts/smoke-roundtrip.sh
./scripts/smoke-inject-goauld.sh   # needs adb device + libgoauld_agent.so
```
