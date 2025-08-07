# Manual Build Guide

## How to Trigger Manual Builds

### For Pull Requests
1. **Automatic Instructions**: When you create a PR, a bot comment will appear with a direct link
2. **Click the link** in the bot comment to go to the Manual Build workflow
3. **Select platforms** you want to build
4. **Artifacts** will be posted back to the PR when complete

### For Any Branch
1. Go to [Actions → Manual Build](../actions/workflows/manual-build.yml)
2. Click **"Run workflow"** (green button)
3. Select your branch from the dropdown
4. Choose platforms to build:
   - ☑️ **Linux x64** (GNU) - Standard Linux binary
   - ☑️ **macOS x64** (Intel Macs) - x86_64 binary for Intel Macs
   - ☑️ **macOS ARM64** (Apple Silicon) - Native M1/M2/M3 binary
   - ☑️ **Windows x64** - Standard Windows executable
   - ☑️ **Docker image** - Containerized application
5. Click **"Run workflow"**

## What Happens Next

1. **Build Status**: Check the Actions tab for build progress
2. **Artifacts**: Download from the workflow run page (available for 30 days)
3. **PR Comments**: If triggered from a PR, results are posted as comments

## Artifact Downloads

### Binaries
- **Format**: `sockudo-{platform}` (e.g., `sockudo-linux-x64`)
- **Usage**: Direct execution after download
- **Platforms**: Linux, macOS, Windows

### Docker Images
- **Format**: `sockudo-docker-{branch}-{timestamp}.tar.gz`
- **Usage**: 
  ```bash
  # Extract and load
  gunzip sockudo-docker-*.tar.gz
  docker load < sockudo-docker-*.tar
  
  # Run
  docker run -p 6001:6001 sockudo:your-tag
  ```

## Build Times

| Platform | Typical Time | Cache Hit |
|----------|--------------|-----------|
| Linux x64 | ~8 minutes | ~3 minutes |
| macOS x64 | ~12 minutes | ~5 minutes |
| macOS ARM64 | ~10 minutes | ~4 minutes |
| Windows x64 | ~15 minutes | ~6 minutes |
| Docker | ~10 minutes | ~4 minutes |

## FAQ

**Q: Why are manual builds separate from CI?**
A: To save CI minutes and allow flexible platform selection only when needed.

**Q: Can I build multiple platforms at once?**
A: Yes! Check multiple platforms in the workflow dispatch form.

**Q: How long are artifacts stored?**
A: 30 days (longer than the default 7 days).

**Q: Can I build from any branch?**
A: Yes, select any branch from the dropdown when triggering manually.