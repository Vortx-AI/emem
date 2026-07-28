# Semantic Kernel + emem MCP Agent

A Microsoft Semantic Kernel kernel that connects to emem over MCP (Streamable HTTP)
and answers questions about real places with signed facts from the shared memory, citing receipts any agent can re-verify.

## Install

```bash
pip install semantic-kernel
```

## Run

```bash
export OPENAI_API_KEY="sk-..."
python emem_mcp_geospatial_agent.py
```

Optional:

```bash
export EMEM_MCP_URL="https://emem.dev/mcp"
```

## What it does

1. Connects to `https://emem.dev/mcp` via Streamable HTTP using
   `MCPStreamableHttpPlugin`.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates a Semantic Kernel kernel with a geospatial verification system message.
4. Resolves South Mumbai to its canonical cell64 address, recalls elevation,
   then verifies the returned `fact_cid` receipt.
5. Shows each step: locate, recall, verify.

## Notes

- No API key needed for emem (reads are anonymous).
- Any Semantic Kernel-compatible model service works — swap `OpenAIChatCompletion`
  for Azure OpenAI, Anthropic Claude, or another provider.
- emem tools are auto-discovered via `MCPStreamableHttpPlugin` — no manual tool
  registration needed.
