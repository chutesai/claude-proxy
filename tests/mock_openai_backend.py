#!/usr/bin/env python3
"""Minimal OpenAI-compatible mock backend for CI and local integration tests."""

from __future__ import annotations

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


MODELS = [
    {
        "id": "zai-org/GLM-4.5-Air",
        "supported_features": [],
        "price": {
            "input": {"usd": 0.1},
            "output": {"usd": 0.2},
        },
    },
    {
        "id": "deepseek-r1",
        "supported_features": ["thinking", "extended_thinking"],
        "price": {
            "input": {"usd": 0.2},
            "output": {"usd": 0.4},
        },
    },
    {
        "id": "anthropic/claude-3-5-sonnet",
        "supported_features": [],
        "price": {
            "input": {"usd": 3.0},
            "output": {"usd": 15.0},
        },
    },
]


def _json_bytes(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode("utf-8")


def _model_exists(model: str) -> bool:
    return any(item["id"].lower() == model.lower() for item in MODELS)


def _is_reasoning_request(payload: dict) -> bool:
    model = str(payload.get("model", "")).lower()
    return (
        "deepseek" in model
        or "reason" in model
        or "r1" in model
        or payload.get("thinking") is not None
    )


def _tool_call_response(payload: dict) -> tuple[list[dict], str]:
    tools = payload.get("tools") or []
    if not tools:
        return [], "stop"

    first_tool = tools[0]["function"]["name"]
    return (
        [
            {
                "id": "call_mock_1",
                "type": "function",
                "function": {
                    "name": first_tool,
                    "arguments": "{\"path\":\"README.md\"}",
                },
            }
        ],
        "tool_calls",
    )


class MockHandler(BaseHTTPRequestHandler):
    server_version = "MockOpenAI/1.0"

    def log_message(self, format: str, *args: object) -> None:
        return

    def _read_json(self) -> dict:
        content_length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(content_length) if content_length else b"{}"
        return json.loads(body.decode("utf-8"))

    def _send_json(self, status: int, payload: object) -> None:
        data = _json_bytes(payload)
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _send_stream(self, events: list[dict]) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()

        for event in events:
            self.wfile.write(b"data: ")
            self.wfile.write(_json_bytes(event))
            self.wfile.write(b"\n\n")
        self.wfile.write(b"data: [DONE]\n\n")

    def do_GET(self) -> None:
        if self.path == "/v1/models":
            self._send_json(200, {"object": "list", "data": MODELS})
            return

        self._send_json(404, {"error": {"message": "not found", "type": "not_found"}})

    def do_POST(self) -> None:
        if self.path != "/v1/chat/completions":
            self._send_json(404, {"error": {"message": "not found", "type": "not_found"}})
            return

        payload = self._read_json()
        model = str(payload.get("model", ""))

        if not _model_exists(model):
            self._send_json(
                404,
                {"error": {"message": f"model '{model}' not found", "type": "not_found"}},
            )
            return

        response_text = "Mock backend response."
        reasoning_text = "I reasoned briefly about the answer." if _is_reasoning_request(payload) else ""
        tool_calls, finish_reason = _tool_call_response(payload)
        usage = {"prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20}

        if payload.get("stream", False):
            events: list[dict] = [
                {
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": None}],
                }
            ]
            if reasoning_text:
                events.append(
                    {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [
                            {
                                "index": 0,
                                "delta": {"reasoning_content": reasoning_text},
                                "finish_reason": None,
                            }
                        ],
                    }
                )
            if tool_calls:
                events.append(
                    {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [
                            {
                                "index": 0,
                                "delta": {
                                    "tool_calls": [
                                        {
                                            "index": 0,
                                            "id": tool_calls[0]["id"],
                                            "type": "function",
                                            "function": {"name": tool_calls[0]["function"]["name"]},
                                        }
                                    ]
                                },
                                "finish_reason": None,
                            }
                        ],
                    }
                )
                events.append(
                    {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [
                            {
                                "index": 0,
                                "delta": {
                                    "tool_calls": [
                                        {
                                            "index": 0,
                                            "function": {
                                                "arguments": tool_calls[0]["function"]["arguments"],
                                            },
                                        }
                                    ]
                                },
                                "finish_reason": None,
                            }
                        ],
                    }
                )
            else:
                events.append(
                    {
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "model": model,
                        "choices": [
                            {
                                "index": 0,
                                "delta": {"content": response_text},
                                "finish_reason": None,
                            }
                        ],
                    }
                )

            events.append(
                {
                    "id": "chatcmpl-mock",
                    "object": "chat.completion.chunk",
                    "model": model,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}],
                    "usage": usage,
                }
            )
            self._send_stream(events)
            return

        message: dict[str, object] = {
            "role": "assistant",
            "content": response_text,
        }
        if reasoning_text:
            message["reasoning_content"] = reasoning_text
        if tool_calls:
            message["tool_calls"] = tool_calls
            message["content"] = ""

        self._send_json(
            200,
            {
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "message": message,
                        "finish_reason": finish_reason,
                    }
                ],
                "usage": usage,
            },
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8000)
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.host, args.port), MockHandler)
    print(f"mock backend listening on http://{args.host}:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
