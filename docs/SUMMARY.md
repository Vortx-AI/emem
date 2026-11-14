# Summary

<!-- Reading map: first steps (Introduction, Start here, Glossary, emem in
     depth, Why an agent benefits, Quickstart, first verified memory). Then
     the differentiators (What only emem does, Connect & evolve). Then Security,
     Protocol, Research, Surfaces, Developers, Operators. A new reader starts
     at the top. -->

[Introduction](./intro.md)
[Start here (10 minutes)](./start-here.md)
[Glossary](./glossary.md)
[emem in depth](./emem-in-depth.md)
[Why an agent benefits from emem](./why-agents.md)
[Quickstart](./quickstart.md)
[Your first verified memory (10 minutes)](./tutorials/first-verified-memory.md)
[What only emem does](./only-emem.md)
[Connect & evolve](./connect-and-evolve.md)

---

# Security

<!-- First, not last. An agent or a reviewer deciding whether to read from us,
     write to us, or cite us to a third party should not have to reconstruct
     the trust model from the protocol spec and four surface pages. This one
     page states what is checked, what is proven, and what we do not claim. -->

- [Security and trust](./security.md)
  - [Threat model: who can change what another agent reads](./security/threat-model.md)

# Protocol

- [Whitepaper](./whitepaper-v3.md)
  - [Whitepaper v2 (superseded)](./whitepaper-v2.md)
  - [Whitepaper v1 (archived)](./whitepaper-v1.md)
- [Protocol](./protocol.md)
- [Registries](./registries.md)
- [Memory standard v1: what a good memory is, and the audit](./standards/memory-v1.md)
- [Federation & scale-out (design)](./federation.md)

# Research

<!-- These pages are the record, including the parts that went against us.
     The program page and its children state each experiment. The benchmark,
     statistics and comparison pages state what we measured and what we refused
     to claim. The collaboration log is the raw signed argument behind them. -->

- [The research program: algorithms for agents, settled by experiment](./research/program.md)
  - [Finding sudden canopy loss from Sentinel-2 cell series](./research/canopy-loss-2026-10.md)
  - [What the research found missing in emem](./research/gaps.md)
  - [Method selection: when the method an agent trusted is wrong for this place](./research/method-selection.md)
  - [An independent reference: a probability sample interpreted by people](./research/reference-sample.md)
- [How emem compares, and what we have not measured](./how-emem-compares.md)
- [Benchmarks and methods](./benchmarks.md)
- [Statistics, cost, and threats to validity](./paper-section-statistics-and-threats.md)
- [The collaboration log](./collaboration-log.md)
- [Case study: three teams and one sugar mill study](./case-studies/sugar-mill.md)

# Surfaces

- [Agents](./agents.md)
- [Machines that ask where they are](./robots.md)
- [Memory substrate](./memory.md)
- [The memory model](./model.md)
  - [Sleep-time agent (opt-in consolidation)](./sleep-agent.md)
- [EUDR DDS + visual evidence](./eudr.md)
- [Integrations](./integrations.md)
  - [emem-langmem (LangChain / LangGraph)](./sdks/langmem.md)
- [MCP Directory](./mcp-directory.md)

# Developers

- [Errors](./errors.md)
- [Self-host](./self-host.md)
- [Limits and roadmap](./roadmap.md)
- [Encoder substrates: the device trust layer](./plans/encoder-substrates.md)
- [Architecture](./developers/architecture.md)
- [Data sources](./developers/data-sources.md)
- [Developing](./developers/developing.md)
- [How to write emem docs](./writing-docs.md)
- [Inference](./developers/inference.md)

# Operators

- [Operating](./operators/operating.md)
- [Deprecations](./operators/deprecations.md)
