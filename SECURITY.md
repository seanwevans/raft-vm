# Security Policy

This document explains how to report security vulnerabilities in Raft, what the
project considers in scope, and how maintainers evaluate and remediate reports.
Raft is a lightweight Rust virtual machine for actor-style concurrency,
message passing, supervision, and a small native standard library.

## Supported Versions

Raft is currently pre-1.0 software. Security fixes are generally made on the
`main` development line and released in the next available version.

| Version | Supported | Notes |
| ------- | --------- | ----- |
| `main` | Yes | Receives security fixes first. |
| `0.x` releases | Best effort | Fixes may be backported when practical. |
| Unreleased forks or modified builds | No | Please reproduce against `main` before reporting. |

If you depend on Raft in production or a security-sensitive environment, pin to
a reviewed commit, monitor upstream changes, and apply security updates quickly.

## Reporting a Vulnerability

Please report suspected vulnerabilities privately. Do not open a public issue,
pull request, discussion, or social media thread with exploit details before the
project has had an opportunity to investigate and coordinate a fix.

Preferred reporting channels, in order:

1. Use GitHub's private vulnerability reporting or create a private security
   advisory for this repository, if enabled.
2. If private reporting is not available, open a public issue that contains only
   a brief, non-sensitive summary and ask maintainers to establish a private
   disclosure channel. Do not include proof-of-concept code, crash payloads,
   secrets, or exploitation instructions in the public issue.

Include as much of the following information as you can safely share:

- Affected commit, tag, or release.
- Operating system, CPU architecture, Rust toolchain version, and build profile.
- Whether the issue affects the CLI, compiler, VM runtime, heap/GC,
  actor/supervision subsystem, native standard-library bindings, or tests.
- Minimal reproduction steps or a minimized `.raft` program.
- Expected behavior and observed behavior.
- Crash logs, sanitizer output, panic messages, or backtraces.
- Impact analysis, including confidentiality, integrity, availability, sandbox
  escape, privilege escalation, or supply-chain implications.
- Whether the vulnerability is already public or under coordinated disclosure
  elsewhere.

## Response and Disclosure Process

Maintainers should make a best-effort attempt to follow this process:

1. **Acknowledge receipt** within 5 business days.
2. **Triage severity** and confirm whether the report is reproducible.
3. **Assign an owner** for investigation and remediation.
4. **Develop and test a fix** in private when exploit details are sensitive.
5. **Prepare coordinated disclosure**, including release notes, advisories, and
   credit if the reporter wants attribution.
6. **Publish the fix** and disclose enough detail for users to assess exposure
   without unnecessarily enabling exploitation of unpatched deployments.

If a report is not considered a security vulnerability, maintainers should
explain the decision and may ask the reporter to convert it into a regular bug
report or enhancement request.

## Scope

The following areas are in scope for security reports:

- Memory-safety issues, undefined behavior, data races, or unsafe behavior in
  Rust code or dependencies.
- VM instruction decoding or execution bugs that allow invalid memory access,
  instruction-pointer corruption, reference-count corruption, or unexpected host
  process termination.
- Compiler or parser bugs that cause crashes, incorrect code generation with
  security impact, or uncontrolled resource consumption.
- Heap, garbage-collection, reference-counting, or object-lifetime defects that
  can be triggered by untrusted `.raft` input or bytecode.
- Actor, mailbox, message-passing, supervision, or restart behavior that enables
  denial of service, message spoofing, lost isolation guarantees, or unintended
  cross-process state access.
- Native standard-library functions or host integrations that expose filesystem,
  network, process, environment, terminal, or other host capabilities without
  explicit intent.
- Dependency vulnerabilities that are reachable from Raft's normal build,
  tests, examples, CLI, compiler, runtime, or standard-library paths.
- Build, release, packaging, or documentation issues that could mislead users
  into insecure deployment.

The following are usually out of scope unless they demonstrate a broader impact:

- Vulnerabilities that require a malicious local user to already have equivalent
  privileges on the same machine.
- Reports against unsupported forks or modified builds that cannot be reproduced
  against upstream `main`.
- Theoretical issues without a plausible trigger or impact.
- Denial-of-service reports based solely on intentionally huge inputs, unless
  they reveal an unbounded algorithmic behavior that affects realistic use.
- Missing security headers, cookie flags, or browser-only controls, because Raft
  is not currently a web application.

## Threat Model

Raft executes programs inside the same operating-system process as the host
application. The VM is intended to provide language-level structure and actor
isolation, not a hardened security sandbox.

Important assumptions:

- Running a `.raft` program is equivalent to asking the host process to evaluate
  potentially expensive input. Treat untrusted programs as untrusted code.
- The current standard library exposes host-backed native functions. Even simple
  native functions such as `io.print` can affect host-visible output and logs.
- Actor isolation is a VM/runtime design boundary. It should not be treated as a
  tenant boundary for mutually hostile users without additional process,
  container, seccomp, cgroup, network, filesystem, and operating-system policy
  controls.
- Bytecode produced by trusted compiler paths is expected. Hand-crafted or
  corrupted bytecode should fail safely, but it is still part of the attack
  surface when accepted by an embedding application.
- Availability is security-relevant. Infinite loops, unbounded actor creation,
  mailbox growth, heap growth, stack growth, excessive logging, and expensive
  arithmetic can exhaust CPU, memory, or disk resources.

## Security Expectations for Embedders

Applications embedding Raft should add controls appropriate to their risk model:

- Run untrusted workloads in a separate OS process, container, VM, or sandbox.
- Apply CPU, memory, file-descriptor, process, thread, and wall-clock limits.
- Bound input size, bytecode size, stack depth, heap size, mailbox size, actor
  count, supervision restart rate, and log volume.
- Disable or restrict native functions that expose host capabilities.
- Treat `.raft` source, bytecode, actor messages, and native-function arguments
  as untrusted input.
- Avoid passing secrets into VM-visible globals, locals, messages, logs, or
  native-function outputs unless disclosure is acceptable.
- Use structured logging and avoid logging attacker-controlled data at high
  volume or in contexts where log injection matters.
- Pin dependency versions, review updates, and rebuild with a supported Rust
  toolchain.

## Secure Development Practices

Contributors should consider the following before submitting changes:

- Prefer safe Rust. Any future `unsafe` code must document invariants and be
  covered by focused tests.
- Validate instruction operands, jump targets, native function arity, heap
  references, and stack preconditions before use.
- Fail closed with typed errors instead of panicking on malformed input.
- Add regression tests for parser, compiler, VM, heap, actor, supervision, and
  native-function security fixes.
- Keep resource usage bounded where practical and document intentional limits.
- Avoid adding host capabilities to the standard library without documenting the
  security model and opt-in behavior.
- Do not print secrets, environment variables, paths, or system details in error
  messages unless necessary for diagnostics.
- Run the test suite before merging security-sensitive changes.

Suggested local checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo audit
```

`cargo audit` requires the external `cargo-audit` tool and may need network
access to update the advisory database.

## Dependency and Supply-Chain Security

Raft depends on third-party Rust crates. Maintainers should:

- Review new dependencies for maintenance status, license compatibility,
  transitive dependency impact, and security history.
- Prefer small, well-maintained dependencies with clear ownership.
- Keep `Cargo.lock` changes reviewable.
- Monitor RustSec advisories and upstream release notes.
- Avoid executing untrusted build scripts or generated code without review.
- Verify release artifacts are built from the intended source revision.

## Handling Secrets

Do not commit secrets, tokens, credentials, private keys, production data, or
sensitive logs to this repository. If a secret is accidentally committed:

1. Revoke or rotate it immediately.
2. Remove it from the repository history if appropriate.
3. Audit logs and dependent systems for misuse.
4. Document the incident privately if it has security impact.

## Vulnerability Severity Guidelines

Severity depends on exploitability, reachability, and impact. The following
examples are guidelines, not strict rules:

- **Critical**: Remote code execution in the host process, sandbox escape from a
  documented isolation boundary, or compromise of release artifacts.
- **High**: Reliable host process crash from untrusted input, unauthorized host
  capability access, severe dependency vulnerability reachable in default use,
  or cross-actor isolation bypass with meaningful data exposure.
- **Medium**: Resource exhaustion requiring moderate attacker control, incorrect
  execution semantics with security impact, or disclosure of limited diagnostic
  information.
- **Low**: Hard-to-trigger crashes, minor information leaks, or defense-in-depth
  issues with limited practical impact.

## Public Disclosure and Credits

The project supports coordinated vulnerability disclosure. Reporters may request
public credit in release notes, advisories, or commits. If you prefer to remain
anonymous, say so in your report.

Please do not publish exploit details until users have had a reasonable
opportunity to upgrade, unless there is active exploitation or another compelling
public-interest reason for earlier disclosure.

## Security Updates

Security fixes may be distributed as commits, release tags, or advisories,
depending on project maturity and severity. Users should monitor the repository
for new releases and security advisories, and should rebuild applications that
embed Raft after applying fixes.
