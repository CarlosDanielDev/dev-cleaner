# dev-cleaner purge manifest
Date: 2026-08-19 · Tier 1 · volumes excluded

| item | bytes_before | status |
|---|---:|---|
| docker prune -a (no volumes) | - | done |
| /Users/carlos/Library/Developer/Xcode/iOS DeviceSupport | 11377 MB | removed |
| /Users/carlos/Library/Developer/Xcode/UserData/Previews | 5458 MB | removed |
| /Users/carlos/Library/Developer/Xcode/UserData/IB%20Support | 665 MB | removed |
| /Users/carlos/Library/Caches/Arc | 1393 MB | removed |
| /Users/carlos/Library/Caches/ms-playwright | 519 MB | removed |
| /Users/carlos/Library/Caches/go-build | 167 MB | removed |
| /Users/carlos/.npm/_cacache | 506 MB | removed |
| /Users/carlos/.cargo/registry/cache | 85 MB | removed |
| /Users/carlos/.cargo/registry/src | 729 MB | removed |
| Xcode/DerivedData/* | 331 MB | removed |
| go clean -modcache | 2900 MB | done |
| brew cleanup -s | 92 MB | done |

**Reclaimed: 41.79 GB** (free 27.4 GB -> 69.2 GB)
