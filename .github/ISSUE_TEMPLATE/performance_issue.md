---
name: Performance Issue
about: Report performance problems on embedded systems
title: "[PERFORMANCE] "
labels: performance, embedded
assignees: ''

---

**Performance Issue Description**
A clear description of the performance problem you're experiencing.

**Target Hardware**
- Device: [e.g. Raspberry Pi 4, Jetson AGX Xavier]
- CPU: [e.g. ARM Cortex-A72, ARM Cortex-A78AE]
- Memory: [e.g. 4GB LPDDR4]
- Storage: [e.g. microSD Class 10, NVMe SSD]

**Performance Metrics**
Please provide relevant metrics:

**CPU Usage:**
- Average: [e.g. 45%]
- Peak: [e.g. 85%]
- Sustained load duration: [e.g. 30 seconds]

**Memory Usage:**
- Baseline: [e.g. 256MB]
- Peak: [e.g. 1.2GB]
- Memory allocation pattern: [e.g. steady growth, spikes]

**Real-time Constraints**
- [ ] Hard real-time requirements (< 1ms)
- [ ] Soft real-time requirements (< 10ms)
- [ ] Best effort (< 100ms)
- [ ] No specific timing requirements

**Test Configuration**
```toml
# If applicable, share your test configuration
[test.performance]
cpu_test_duration = "200ms"
memory_bandwidth_iterations = 1000
# ... other relevant settings
```

**Expected vs Actual Performance**
- Expected: [e.g. < 50% CPU usage]
- Actual: [e.g. > 80% CPU usage]

**Reproduction Steps**
1. Run on [hardware]
2. Execute [specific command or test]
3. Monitor with [tool/method]
4. Observe [specific performance issue]

**Profiling Data**
If you have profiling data, please attach or paste relevant sections:

```
paste profiling output here
```

**Impact Assessment**
- [ ] Blocks deployment to production
- [ ] Degrades user experience
- [ ] Causes system instability
- [ ] Acceptable but suboptimal

**Additional Context**
Any other information that might help diagnose the performance issue.