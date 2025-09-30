# Binary Testing with Docker

This directory contains Docker-based tests for different Sockudo binary variants.

## Prerequisites

- Docker installed with BuildKit support
- Docker configured for multi-platform builds (for ARM64 testing on x86_64 hosts)

## Setup

1. **Download the tar archives** from GitHub Actions and place them in this directory:
   - `sockudo-x86_64-unknown-linux-gnu.tgz`
   - `sockudo-x86_64-unknown-linux-musl.tgz`
   - `sockudo-aarch64-unknown-linux-gnu.tgz`
   - `sockudo-aarch64-unknown-linux-musl.tgz`
   
   Note: Each archive contains a binary named `sockudo`

2. **Enable Docker multi-platform support** (if testing ARM on x86):
   ```bash
   docker run --rm --privileged multiarch/qemu-user-static --reset -p yes
   ```

## Testing

### Test all binaries at once:
```bash
./test-all.sh
```

### Test individual binaries:

**x86_64 GNU (Ubuntu-based):**
```bash
docker build -f Dockerfile.x86_64-gnu -t sockudo-test-x86-gnu .
docker run --rm sockudo-test-x86-gnu
```

**x86_64 musl (Alpine-based):**
```bash
docker build -f Dockerfile.x86_64-musl -t sockudo-test-x86-musl .
docker run --rm sockudo-test-x86-musl
```

**ARM64 GNU (Ubuntu-based):**
```bash
docker build --platform linux/arm64 -f Dockerfile.aarch64-gnu -t sockudo-test-arm64-gnu .
docker run --rm --platform linux/arm64 sockudo-test-arm64-gnu
```

**ARM64 musl (Alpine-based):**
```bash
docker build --platform linux/arm64 -f Dockerfile.aarch64-musl -t sockudo-test-arm64-musl .
docker run --rm --platform linux/arm64 sockudo-test-arm64-musl
```

## Running a Full Server Test

To test the binary as a running server, modify the CMD in the Dockerfile:

```dockerfile
# Instead of:
CMD ["/usr/local/bin/sockudo", "--version"]

# Use:
CMD ["/usr/local/bin/sockudo", "--host", "0.0.0.0", "--port", "6001"]
```

Then run with port mapping:
```bash
docker run --rm -p 6001:6001 sockudo-test-x86-gnu
```

## Troubleshooting

**"exec format error"**: The binary architecture doesn't match the container platform. Ensure you're using the correct --platform flag.

**Missing libraries**: The GNU binaries may need additional libraries. The Dockerfiles include minimal dependencies, but you may need to add more based on your build configuration.

**Permission denied**: Ensure the binaries have execute permissions before copying them to the container.

## Notes

- The GNU variants use Ubuntu base images with glibc
- The musl variants use Alpine Linux with musl libc
- All containers include ca-certificates for HTTPS support
- The test runs `--version` by default to verify the binary works