# Contributing to Renoir

Thank you for your interest in contributing to Renoir! This document provides guidelines for contributing to our high-performance embedded systems library.

## 🤝 How to Contribute

We welcome contributions of all kinds:

- 🐛 **Bug Reports**: Help us identify and fix issues
- 🚀 **Feature Requests**: Suggest new functionality for embedded systems
- 💾 **Performance Optimizations**: Improve performance on target hardware
- 📚 **Documentation**: Improve docs, examples, and guides
- 🧪 **Tests**: Add test coverage for new platforms or edge cases
- 🔧 **Code Contributions**: Implement features and fixes

## 🛠️ Development Setup

### Prerequisites

- **Rust**: Version 1.75.0 or later
- **Git**: For version control
- **Target Hardware**: Access to Raspberry Pi or Jetson AGX Xavier (optional but helpful)

### Setting Up Your Environment

1. **Fork and Clone**
   ```bash
   git clone https://github.com/your-username/renoir.git
   cd renoir
   ```

2. **Install Development Tools**
   ```bash
   # Install required tools
   cargo install cargo-audit cargo-deny cargo-geiger
   cargo install cargo-criterion cargo-tarpaulin
   
   # Install cross-compilation targets
   rustup target add aarch64-unknown-linux-gnu
   rustup target add armv7-unknown-linux-gnueabihf
   rustup target add x86_64-unknown-linux-musl
   ```

3. **Install Cross-compilation Toolchains**
   ```bash
   # Ubuntu/Debian
   sudo apt-get install gcc-aarch64-linux-gnu gcc-arm-linux-gnueabihf
   
   # Or install cross for easier cross-compilation
   cargo install cross
   ```

4. **Verify Setup**
   ```bash
   # Run all tests
   cargo test --release
   
   # Run security checks
   cargo audit && cargo deny check
   
   # Test cross-compilation
   cargo build --target aarch64-unknown-linux-gnu --release
   ```

## 📋 Development Workflow

### 1. Create a Branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/issue-number-description
```

### 2. Make Changes

Follow our coding standards:

- **Code Style**: Use `cargo fmt` for consistent formatting
- **Linting**: Ensure `cargo clippy` passes without warnings
- **Testing**: Add tests for new functionality
- **Documentation**: Update docs for public APIs

### 3. Test Thoroughly

```bash
# Run all tests
cargo test --release

# Run specific test categories
cargo test cpu_performance_tests --release
cargo test memory_performance_tests --release
cargo test topic_manager_tests --release

# Test with limited threads (embedded simulation)
RUST_TEST_THREADS=1 cargo test --release

# Run benchmarks
cargo bench

# Check test coverage
cargo tarpaulin --all-features --workspace
```

### 4. Security and Quality Checks

```bash
# Security audit
cargo audit

# License and dependency checks
cargo deny check

# Unsafe code analysis
cargo geiger

# Format code
cargo fmt

# Lint code
cargo clippy -- -D warnings
```

### 5. Cross-compilation Testing

```bash
# Test ARM64 compilation (Raspberry Pi 4, Jetson AGX Xavier)
cargo build --target aarch64-unknown-linux-gnu --release

# Test ARM7 compilation (older Raspberry Pi models)
cargo build --target armv7-unknown-linux-gnueabihf --release

# Test musl target (static linking)
cargo build --target x86_64-unknown-linux-musl --release
```

### 6. Commit and Push

```bash
git add .
git commit -m "feat: add new embedded optimization for ARM64"
# or
git commit -m "fix: resolve memory bandwidth test hanging on Pi 4"

git push origin your-branch-name
```

### 7. Create Pull Request

1. Go to GitHub and create a Pull Request
2. Fill out the PR template completely
3. Link any related issues
4. Request review from maintainers

## 🎯 Contribution Guidelines

### Code Quality Standards

- **Performance First**: All code must be optimized for embedded systems
- **Memory Efficiency**: Bounded allocations, no unbounded growth
- **Real-time Friendly**: Avoid blocking operations in critical paths
- **Cross-platform**: Must work on ARM64, ARM7, and x86_64
- **Well Tested**: Minimum 80% test coverage for new code
- **Documented**: Public APIs must have comprehensive documentation

### Embedded Systems Considerations

When contributing, keep in mind our target platforms:

- **Raspberry Pi 4**: 4GB RAM, ARM Cortex-A72
- **Jetson AGX Xavier**: 32GB RAM, ARM Cortex-A78AE  
- **Resource Constraints**: Code must work efficiently on 4GB systems
- **Real-time Requirements**: Sub-millisecond response times
- **Power Efficiency**: Battery-powered deployment scenarios

### Test Requirements

All contributions must include appropriate tests:

1. **Unit Tests**: Test individual functions and modules
2. **Integration Tests**: Test component interactions
3. **Performance Tests**: Validate performance characteristics
4. **Embedded Tests**: Test on actual or simulated embedded hardware
5. **Cross-compilation Tests**: Ensure code compiles for all targets

### Performance Benchmarking

For performance-related contributions:

```bash
# Run benchmarks before changes
cargo bench > baseline.txt

# Make your changes

# Run benchmarks after changes  
cargo bench > optimized.txt

# Compare results
# Include performance impact in PR description
```

## 📝 Commit Message Guidelines

We follow conventional commits:

- `feat:` - New features
- `fix:` - Bug fixes
- `perf:` - Performance improvements
- `test:` - Adding or updating tests
- `docs:` - Documentation changes
- `refactor:` - Code refactoring
- `ci:` - CI/CD changes
- `chore:` - Maintenance tasks

Examples:
```
feat: add ARM NEON optimizations for memory bandwidth tests
fix: resolve topic manager ring buffer capacity validation
perf: optimize CPU utilization for sustained load scenarios  
test: add cross-compilation tests for Jetson AGX Xavier
docs: improve embedded deployment guide with Pi 4 examples
```

## 🧪 Testing on Real Hardware

### Raspberry Pi Testing

If you have access to a Raspberry Pi:

```bash
# Build for Pi
cargo build --target aarch64-unknown-linux-gnu --release

# Copy to Pi  
scp target/aarch64-unknown-linux-gnu/release/renoir pi@192.168.1.100:~/

# Run tests on Pi
ssh pi@192.168.1.100 'cd ~ && ./renoir test --release'
```

### Performance Validation

Validate performance on target hardware:

```bash
# Monitor system resources
htop & 
free -h

# Run performance tests
cargo test cpu_performance_tests --release -- --nocapture
cargo test memory_performance_tests --release -- --nocapture

# Profile memory usage
valgrind --tool=massif ./target/release/renoir
```

## 🚀 Types of Contributions

### 🐛 Bug Reports

When reporting bugs, please include:

- Target hardware (Pi 4, Jetson, etc.)
- Rust version and target triple
- Steps to reproduce
- Expected vs actual behavior
- System resource usage (CPU, memory)
- Error logs with RUST_LOG=debug

### 🚀 Feature Requests

For new features, consider:

- Embedded system impact
- Memory usage implications
- Real-time performance effects
- Cross-platform compatibility
- Power consumption impact

### 💾 Performance Optimizations

Performance improvements are always welcome:

- ARM NEON/SVE optimizations
- Cache-friendly data structures
- Lock-free algorithms
- Memory layout optimizations
- SIMD implementations

### 📚 Documentation

Documentation improvements include:

- API documentation
- Embedded deployment guides  
- Performance optimization tutorials
- Hardware-specific configuration
- Example applications

## 🔍 Review Process

### What We Look For

1. **Code Quality**: Clean, readable, well-structured code
2. **Performance**: Maintains or improves performance characteristics
3. **Testing**: Comprehensive test coverage
4. **Documentation**: Clear documentation for new features
5. **Compatibility**: Works across all supported platforms
6. **Security**: No security vulnerabilities introduced

### Review Timeline

- **Initial Review**: Within 48 hours
- **Feedback Incorporation**: Ongoing collaboration
- **Final Approval**: Once all requirements are met
- **Merge**: After CI passes and approval is granted

## 🆘 Getting Help

If you need help:

1. **Check Documentation**: README.md and docs/ directory
2. **Search Issues**: Look for existing discussions
3. **Create Discussion**: For questions and ideas
4. **Join Community**: Follow project updates
5. **Ask Maintainers**: Tag maintainers in issues

## 🏷️ Release Process

Releases follow semantic versioning:

- **Major** (1.0.0): Breaking changes
- **Minor** (0.1.0): New features, backwards compatible
- **Patch** (0.0.1): Bug fixes, backwards compatible

## 🙏 Recognition

Contributors are recognized through:

- GitHub contributors page
- Release changelogs
- Documentation acknowledgments
- Community highlights

Thank you for contributing to Renoir! Your help makes high-performance embedded systems development accessible to everyone.