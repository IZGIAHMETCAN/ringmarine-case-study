# 0001 — Move the middleware from ECS/ALB to Lambda behind an HTTP API

- **Status:** accepted, shipped
- **Date:** 2026

## Context

The middleware started on ECS Fargate behind an Application Load Balancer, because that is the shape most teams
reach for and it removes a whole class of "does the runtime allow this?" questions.

The traffic profile makes that expensive. A marina is busy in the morning and on summer weekends, and idle for
long stretches. The service has to be *available* 24/7 — a boat owner who cannot switch the water off at 23:00 is
a support ticket — but it does not have to be *running* 24/7. With ECS that distinction does not exist: the task
is either up (costing money) or down (costing requests). We were paying for a fixed floor of compute in order to
serve a workload whose median minute is empty.

Two other factors mattered:

- The handler logic was already a pure HTTP function. Nothing in it held in-memory state between requests, because
  it could not assume it would hit the same task twice.
- The load profile is spiky and per-marina, not smooth and global. Capacity planning meant sizing for the worst
  marina at its worst hour.

## Decision

Move to **AWS Lambda behind API Gateway (HTTP API)**.

- Keep the Axum router and extract it into a shared app builder, so the same routes serve both runtimes. The
  runtime adapter (`lambda_http`) becomes the only platform-specific code.
- Resolve configuration and credentials at cold start, not per request.
- Set the request timeout to 28s, below the API Gateway integration limit (29s), so the middleware returns a
  structured error instead of an opaque gateway timeout.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Keep ECS, scale down at night on a schedule | Marinas are not on a shared schedule, and the floor cost only drops by half |
| ECS with scale-to-zero (KEDA/Application Auto Scaling to 0) | Possible, but cold starts were worse than Lambda's and the operational surface is larger |
| EC2 with a tiny instance | Cheapest on paper, but it is a server to patch, monitor and lose |
| Keep ECS for the payment path only | Two deployments, two config paths, one shared idempotency store — more failure modes than it removes |

## Consequences

**Good**

- Fixed compute cost went to **zero**. The bill is now a function of actual requests.
- Scaling is per-request, so one busy marina cannot starve another.
- No patching, no container base image, no ALB to configure.

**Bad / accepted costs**

- **Cold starts.** Release profile uses `lto = true`, `codegen-units = 1`, `opt-level = 3`; a cold start adds
  latency to the first request after idle. Acceptable because marina traffic is bursty rather than latency-critical,
  but it is a real regression against a warm container.
- **Config fetch on cold start.** Every cold start pays for the parameter-store round trip. Mitigated by fetching
  the whole path in one call rather than per parameter.
- **Per-invocation pricing has a crossover.** Above sustained high volume, Fargate wins. We are far below it; the
  decision is revisited if a large marina group ever runs every berth through this.
- **No connection reuse across invocations** for the outbound legacy backend. This is the trade we made knowingly.

## What I would do differently

Use **provisioned concurrency on a small baseline** from the start for the payment endpoints specifically, instead
of treating cold start as a globally acceptable cost. Payments are the one path where an extra 400ms is user-visible
and support-generating, and splitting the handler into two functions with different concurrency settings would have
cost less than the support tickets did.
