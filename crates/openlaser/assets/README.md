# Windows webview setup

`MicrosoftEdgeWebview2Setup.exe` is Microsoft's signed Evergreen bootstrapper.
It is a local build input, excluded from Git; this repository ships no executable
downloads. Windows `just standalone` and `just check` fetch it automatically.
For a Windows cross-build, prepare it explicitly from the repository root:

```sh
node scripts/prepare-webview2.mjs --force
```

The script pins the exact Microsoft CDN URL resolved from
https://go.microsoft.com/fwlink/p/?LinkId=2124703 on 2026-09-17 and verifies
SHA-256 `83004a28553bcf2f932bf03564fbab407b8e1f59cd265f8dc99cc53d028e459c`.
If Microsoft changes the file,
the build setup stops until the replacement and checksum are reviewed.
It is embedded only in Windows desktop builds. OpenLaser also checks the
publisher and Authenticode signature before running it when WebView2 is missing.

The runtime is shared and maintained by Microsoft. It is not part of OpenLaser's
GPL source. Microsoft's distribution instructions and terms are available at:

- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
- https://developer.microsoft.com/en-us/microsoft-edge/webview2/

The bootstrapper needs internet when the runtime is absent. An offline machine
can have the Evergreen Runtime installed before OpenLaser is launched.
