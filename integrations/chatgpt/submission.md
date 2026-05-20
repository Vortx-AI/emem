# ChatGPT App Submission Notes

## For reviewers

emem is a public, open-source Earth memory layer for AI agents.

It helps ChatGPT answer place-based questions with signed geospatial facts instead of relying only on web search or model reasoning.

The app is read-only. It does not modify user data or take external write actions.

## Links

- Homepage: https://emem.dev
- Repository: https://github.com/Vortx-AI/emem
- MCP endpoint: https://emem.dev/mcp
- Privacy: https://github.com/Vortx-AI/emem/blob/main/integrations/chatgpt/privacy.md
- Support: jaya@vortx.ai

## Listed on

- Official MCP Registry: io.github.Vortx-AI/emem
- Glama: https://glama.ai/mcp/servers/Vortx-AI/emem
- Smithery: https://smithery.ai/servers/vortxai/emem
- HuggingFace Space: https://huggingface.co/spaces/vortx-ai/emem
- GHCR: ghcr.io/vortx-ai/emem:0.0.6

## Technical

- Transport: Streamable HTTP (MCP 2025-03-26)
- Authentication: None (anonymous reads)
- 50 MCP tools available, 4 exposed in this app
- Pure Rust server, Apache-2.0 licensed

## Status

ChatGPT app directory publication depends on OpenAI review and approval. Custom apps can be submitted for publication, and approved apps become available to eligible users.
