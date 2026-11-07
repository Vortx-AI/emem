# AutoGen + emem MCP Agent

A Microsoft AutoGen assistant that connects to emem over MCP (Streamable HTTP)
and answers questions about real places with signed facts from the shared memory, citing receipts any agent can re-verify.

## Install

```bash
pip install autogen-agentchat 'autogen-ext[openai,mcp]' 'mcp<2'
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
   `McpWorkbench` and `StreamableHttpServerParams`.
2. Auto-discovers all emem MCP tools (locate, recall, compare, verify, etc.).
3. Creates an AutoGen `AssistantAgent` with a geospatial verification system message.
4. Asks the agent to resolve South Mumbai, recall its elevation, and
   verify the receipt.
5. The agent shows each step: locate, recall, verify.

## Notes

- No API key needed for emem (reads are anonymous).
- Any AutoGen-compatible model client works -- swap `OpenAIChatCompletionClient`
  for another provider.
