# MicroVM Execution — Architecture Justification & Sizing Guide

## Why MicroVMs Instead of Docker or Bare Processes?

The decision to use MicroVMs (Firecracker, E2B) instead of traditional Docker
containers or native processes is a critical design choice driven by three
imperatives when working with development agents in remote, multi-tenant
environments.

### 1. Absolute Security Against Malicious Code (Sandboxing)

Development agents can generate, modify, and execute arbitrary code via tools
like Bash.

**The Docker danger:** Traditional containers share the host OS kernel. If an
agent suffers a severe hallucination, is attacked, or generates harmful code,
it could find a vulnerability to "escape" the container (container breakout),
accessing other tenants' data or taking control of the host server.

**The MicroVM solution:** A MicroVM provides hardware-level isolation (real
virtualisation). OpenCode runs inside a completely sealed environment with its
own separate kernel. It is technically impossible for the agent to escape this
jail and crash the server or compromise multi-tenant security.

### 2. Millisecond Cold Start (No More Cold Start Problem)

Traditional VMs (VirtualBox, VMware) take minutes to boot because they
virtualise an entire heavy OS, network devices, BIOS, etc.

MicroVMs like Firecracker are minimalised to the extreme. They dispense with
useless virtual devices and are optimised for a single task.

**Result:** A MicroVM boots in < 150ms. This lets us treat OpenCode as an ideal
ephemeral environment: when a Pub/Sub event arrives, the MicroVM spins up
instantly, the agent works, and when done, the MicroVM is destroyed
immediately. The frontend user notices no latency.

### 3. Minimal Resource Consumption (Extreme Efficiency)

For a multi-tenant system to be profitable, you must fit hundreds or thousands
of concurrent agent sessions on the same physical machine without exhausting
RAM.

A classical VM needs gigabytes of RAM just to exist.

A MicroVM consumes a minimal fraction of memory (a few megabytes above the
OpenCode process) and has near-imperceptible CPU overhead at rest. This enables
massive horizontal scaling of parallel agents at a trivial infrastructure cost.

### Summary

MicroVMs give us the best of both worlds: the **infrangible security and
isolation** of a traditional VM, combined with the **instant speed and
lightness** of a Docker container. It is the key piece that makes ephemeral,
amnesic, and 100% secure OpenCode execution possible.

---

## Runtime Abstraction

Conductor abstracts the execution backend behind the `AgentRuntime` trait:

```
src/runtime/
├── mod.rs          # AgentRuntime trait + factory
├── local.rs        # LocalProcessBackend (development)
└── microvm.rs      # MicroVmBackend (Firecracker / E2B)
```

Switch backends via `CONDUCTOR_RUNTIME`:

| Value | Backend | Use Case |
|---|---|---|
| `local` (default) | `LocalProcessBackend` | Development, single-tenant |
| `microvm` | `MicroVmBackend` (Firecracker) | Production multi-tenant, self-hosted |
| `e2b` | `MicroVmBackend` (E2B) | Production multi-tenant, managed cloud |

---

## Host Machine Sizing

The sizing of the "mother machine" depends on the volume of simultaneous
concurrency (how many agents operate at the exact same second).

### Per-Session Resource Consumption (1 agent executing code)

| Component | RAM | CPU |
|---|---|---|
| Rust orchestrator | ~20-50 MB | < 1% |
| MicroVM (Firecracker + OpenCode) | ~150-250 MB | 0.1 vCPU base |
| Skill execution (lint, test, build) | 500 MB - 1 GB spikes | 1 vCPU bursts |

**Rule of thumb:** Reserve **1.5 GB RAM and 0.5 vCPU** per concurrent agent.

### Sizing Profiles

#### Profile A: Development / MVP (5-10 concurrent agents)

| Resource | Value |
|---|---|
| vCPUs | 4 |
| RAM | 8-16 GB |
| Storage | 50 GB SSD |
| Cloud equivalent | AWS t3.xlarge / DigitalOcean 4 vCPU / 16 GB |

#### Profile B: Small Production / Team (30-40 concurrent agents)

| Resource | Value |
|---|---|
| vCPUs | 16 |
| RAM | 32-64 GB |
| Storage | 100 GB NVMe SSD (high IOPS for Git) |
| Cloud equivalent | AWS c6i.4xlarge |

#### Profile C: Industrial Scale (100+ concurrent agents)

At this scale, **scale horizontally** instead of vertically:

- **Control plane:** Dedicated machine (8 vCPU / 16 GB) for the Rust
  orchestrator (API + WebSockets).
- **Workers:** MicroVMs spawn on a separate cluster (E2B managed, or bare-metal
  pool with Kata Containers + K3s).

### Non-Negotiable: Nested Virtualization

To run Firecracker or Kata Containers at native speed, the host must support
**hardware virtualisation**:

- **On-premise:** CPU must have Intel VT-x or AMD-V enabled in BIOS.
- **Cloud:** Choose `.metal` instances or families that explicitly support
  nested virtualisation. Software emulation causes drastic performance drops.
