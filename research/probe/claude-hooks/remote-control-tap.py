import json
import os
import time

from mitmproxy import http

OUT = os.environ.get("TAP_OUT", "flows.jsonl")
INJECT = os.environ.get("TAP_INJECT", "inject.txt")
SECRET = {"authorization", "cookie", "set-cookie", "x-api-key", "proxy-authorization"}


def _log(rec):
    rec["t"] = round(time.time(), 3)
    with open(OUT, "a") as f:
        f.write(json.dumps(rec) + "\n")


def _headers(h):
    return {k: ("<redacted>" if k.lower() in SECRET else v) for k, v in h.items()}


def _body(raw, limit):
    if raw is None:
        return None
    s = raw.decode("utf-8", "replace")
    return s if len(s) <= limit else s[:limit] + f"...<{len(s)} bytes>"


def _limit(flow):
    return 4000 if "/v1/messages" in flow.request.path else 200000


def request(flow: http.HTTPFlow):
    _log({"k": "req", "id": flow.id, "m": flow.request.method, "u": flow.request.pretty_url,
          "h": _headers(flow.request.headers), "b": _body(flow.request.raw_content, _limit(flow))})


def responseheaders(flow: http.HTTPFlow):
    ct = flow.response.headers.get("content-type", "")
    if "event-stream" not in ct:
        return
    fid, url = flow.id, flow.request.pretty_url
    _log({"k": "sse-open", "id": fid, "u": url, "s": flow.response.status_code})

    def stream(chunk: bytes) -> bytes:
        if chunk:
            _log({"k": "sse", "id": fid, "u": url, "b": _body(chunk, _limit(flow))})
        if "/v1/messages" not in url and os.path.exists(INJECT):
            extra = open(INJECT, "rb").read()
            os.remove(INJECT)
            _log({"k": "inject", "id": fid, "u": url, "b": extra.decode()})
            return chunk + extra
        return chunk

    flow.response.stream = stream


def response(flow: http.HTTPFlow):
    if flow.response.stream:
        _log({"k": "sse-close", "id": flow.id, "u": flow.request.pretty_url})
        return
    _log({"k": "res", "id": flow.id, "u": flow.request.pretty_url, "s": flow.response.status_code,
          "b": _body(flow.response.get_content(strict=False), _limit(flow))})
