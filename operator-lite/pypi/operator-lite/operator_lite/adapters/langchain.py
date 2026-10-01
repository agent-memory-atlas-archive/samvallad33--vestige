"""LangChain / LangGraph adapter (port of operator-lite/ports/langchain/
operator_middleware.py onto operator_lite._core).

The gate is spawned as a subprocess for every shell-ish tool call; this
module is payload shaping and decision plumbing only.

    gate exit 0 -> allow        (the wrapped handler runs normally)
    gate exit 2 -> block        (a ToolMessage is returned instead; the
                                 handler is never called, and the LLM sees
                                 "Operator Lite blocked: <gate stderr>")
    spawn error -> fail-open, UNLESS the command matches the destructive
                   safety net, in which case fail-closed

Blocking form per the LangChain v1 middleware API (verified in the port,
2026-09): return a ToolMessage from wrap_tool_call and call the handler zero
times. Attach via create_agent(model, tools, middleware=[operator_middleware()]).

The langchain import happens lazily on first use; importing this module (and
operator_lite) stays langchain-free.
"""
import asyncio
import os

from .. import _core

SOURCE = "langchain"


def _langchain_bits():
    from langchain.agents.middleware import AgentMiddleware
    from langchain_core.messages import ToolMessage
    return AgentMiddleware, ToolMessage


def _build_middleware_class():
    AgentMiddleware, ToolMessage = _langchain_bits()

    def _decide(request):
        call = dict(getattr(request, "tool_call", None) or {})
        return _core.decide(call.get("name", ""), call.get("args") or {},
                            cwd=os.getcwd())

    class OperatorLiteMiddleware(AgentMiddleware):
        """Gate every shell-ish tool call through Operator Lite.

        wrap_tool_call signature and ToolMessage short-circuit per
        https://reference.langchain.com/python/langchain/agents/middleware/types/wrap_tool_call
        """

        def wrap_tool_call(self, request, handler):
            verdict, message = _decide(request)
            if verdict == _core.BLOCK:
                call = dict(getattr(request, "tool_call", None) or {})
                return ToolMessage(content=message,
                                   tool_call_id=str(call.get("id") or ""))
            return handler(request)

        async def awrap_tool_call(self, request, handler):
            loop = asyncio.get_running_loop()
            verdict, message = await loop.run_in_executor(
                None, lambda: _decide(request))
            if verdict == _core.BLOCK:
                call = dict(getattr(request, "tool_call", None) or {})
                return ToolMessage(content=message,
                                   tool_call_id=str(call.get("id") or ""))
            return await handler(request)

    OperatorLiteMiddleware.__module__ = __name__
    return OperatorLiteMiddleware


def __getattr__(name):
    """PEP 562: OperatorLiteMiddleware materialises on first attribute
    access; importing this module stays langchain-free."""
    if name == "OperatorLiteMiddleware":
        cls = _build_middleware_class()
        globals()["OperatorLiteMiddleware"] = cls
        return cls
    raise AttributeError("module %r has no attribute %r" % (__name__, name))


def operator_middleware():
    """Return an OperatorLiteMiddleware instance for create_agent's
    middleware=[...] list. First call imports langchain (lazily)."""
    cls = globals().get("OperatorLiteMiddleware") or __getattr__(
        "OperatorLiteMiddleware")
    return cls()
