#!/usr/bin/env python3
"""SOP-10811 protocol harness: streams a 16 kHz mono WAV through the Google Live socket at
real-time pace using the same turn rules as crates/gemini-transcription/src/live.rs (v1.5),
and reports how long each final takes after activityEnd. The key is read from a file and
is never printed. usage: gemini-live-sim.py wav keyfile [--pace 1.0] [--quiet-ms 1500] [--min-ms 3000]
(--min-ms 99999999 sends the whole clip as one turn = the reference transcript.)"""
import json, base64, time, wave, ssl, argparse, array
import websocket

ap = argparse.ArgumentParser()
ap.add_argument("wav"); ap.add_argument("key")
ap.add_argument("--pace", type=float, default=1.0)
ap.add_argument("--quiet-ms", type=int, default=1500)
ap.add_argument("--min-ms", type=int, default=3000)
ap.add_argument("--json", default=None)
ap.add_argument("--max-s", type=float, default=0)
ap.add_argument("--quiet", action="store_true")
a = ap.parse_args()
w = wave.open(a.wav)
assert w.getframerate() == 16000 and w.getnchannels() == 1
pcm = array.array("h"); pcm.frombytes(w.readframes(w.getnframes()))
if a.max_s: pcm = pcm[: int(a.max_s * 16000)]
FRAME = 512
key = open(a.key).read().strip()
URL = "wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
ws = websocket.create_connection(URL, header=[f"x-goog-api-key: {key}"], timeout=10,
                                 sslopt={"cert_reqs": ssl.CERT_REQUIRED})
T0 = time.time()
def now(): return time.time() - T0
log = []
def L(kind, **kw):
    kw["t"] = round(now(), 3); kw["ev"] = kind; log.append(kw)
    if not a.quiet:
        print(f"{kw['t']:8.3f} {kind:10s} " + " ".join(f"{k}={v}" for k, v in kw.items() if k not in ("t", "ev")), flush=True)
ws.send(json.dumps({"setup": {"model": "models/gemini-3.5-transcribe-live",
    "generationConfig": {"responseModalities": ["TEXT"]},
    "realtimeInputConfig": {"automaticActivityDetection": {"disabled": True}, "activityHandling": "NO_INTERRUPTION"},
    "inputAudioTranscription": {"mode": "VERBATIM", "customVocabulary": []}}}))
while True:
    m = json.loads(ws.recv())
    if "setupComplete" in m: break
    print("setup msg", str(m)[:200])
L("setup")
ws.settimeout(0.002)
ws.send(json.dumps({"realtimeInput": {"activityStart": {}}}))
st = dict(end_at=None, finals=0, base=0, tentative="", complete=False, need_final=False, pending=[], wait=False)
cut = dict(since=0, quiet=0)
MIN = a.min_ms * 16; QUIET = a.quiet_ms * 16
ends = []
pos = 0
texts = []

def speech(fr):
    return len(fr) and sum((x / 32767.0) ** 2 for x in fr) / len(fr) > 0.0001

def recv_all():
    for _ in range(16):
        try:
            raw = ws.recv()
        except websocket.WebSocketTimeoutException:
            return
        m = json.loads(raw); c = m.get("serverContent", {}); st["lastmsg"] = now()
        kinds = list(c.keys()) + [k for k in m.keys() if k != "serverContent"]
        since = round(now() - st["end_at"], 3) if st["end_at"] else None
        txt = c.get("inputTranscription", {}).get("text")
        itx = c.get("interimInputTranscription", {}).get("text")
        if "voiceActivity" in m or "generationComplete" in c or "turnComplete" in c: print("RAW", json.dumps(m)[:300], flush=True)
        L("ws_in", kinds=",".join(kinds), since_end=since, final=txt, interim_chars=(len(itx) if itx else None))
        if m.get("voiceActivity", {}).get("type") == "ACTIVITY_END" and st["end_at"]: st["ack"] = now()
        if c.get("generationComplete"): st["gen"] = True
        if "error" in m:
            L("ERROR", body=str(m)[:300]); raise SystemExit(1)
        if itx is not None: st["tentative"] = itx
        if txt is not None:
            st["finals"] += 1; st["need_final"] = False; st["tentative"] = ""; texts.append(txt)
            if ends and ends[-1].get("final_ms") is None and st["end_at"]:
                ends[-1]["final_ms"] = round((now() - st["end_at"]) * 1000)
        if c.get("turnComplete"):
            st["complete"] = True
            if ends and ends[-1].get("tc_ms") is None and st["end_at"]:
                ends[-1]["tc_ms"] = round((now() - st["end_at"]) * 1000)

def send_audio(fr):
    ws.settimeout(10)
    ws.send(json.dumps({"realtimeInput": {"audio": {"mimeType": "audio/pcm;rate=16000",
        "data": base64.b64encode(array.array("h", fr).tobytes()).decode()}}}))
    ws.settimeout(0.002)

def audio(fr):
    if st["wait"]:
        st["pending"].append(fr); return
    sp = speech(fr); st["need_final"] |= bool(sp)
    st["complete"] = False
    send_audio(fr)
    cut["since"] += len(fr); cut["quiet"] = 0 if sp else cut["quiet"] + len(fr)
    if cut["since"] >= MIN and cut["quiet"] >= QUIET:
        cut["since"] = 0; cut["quiet"] = 0
        if st["need_final"]:
            ws.send(json.dumps({"realtimeInput": {"activityEnd": {}}}))
            st["end_at"] = now(); st["wait"] = True; st["base"] = st["finals"]; st["ack"] = None; st["gen"] = False
            ends.append(dict(end_s=round(now(), 2), audio_s=round(pos / 16000, 2), final_ms=None, tc_ms=None))
            L("activityEnd", audio_s=round(pos / 16000, 2))

def poll():
    recv_all()
    empty = st["wait"] and st["finals"] == st["base"] and not st["tentative"] and st.get("ack") and now() - st["ack"] >= 3
    if st["wait"] and ((st["finals"] > st["base"] and not st["tentative"]) or empty):
        if empty: st["need_final"] = False; L("empty_turn")
        st["wait"] = False
        ws.send(json.dumps({"realtimeInput": {"activityStart": {}}})); L("activityStart")
        pend, st["pending"] = st["pending"], []
        for f in pend: audio(f)

nxt = time.time(); dt = FRAME / 16000 / a.pace
while pos + FRAME <= len(pcm):
    time.sleep(max(0, nxt - time.time())); nxt += dt
    fr = list(pcm[pos: pos + FRAME]); pos += FRAME
    audio(fr); poll()
L("release", audio_s=round(pos / 16000, 2), waiting=st["wait"], need_final=st["need_final"])
rel = now()
dl = now() + 40
while st["wait"] and now() < dl: poll(); time.sleep(0.002)
ws.send(json.dumps({"realtimeInput": {"activityEnd": {}}})); st["end_at"] = now(); st["complete"] = False; st["ack"] = None; st["gen"] = False; st["last"] = now()
ends.append(dict(end_s=round(now(), 2), audio_s=round(pos / 16000, 2), final_ms=None, tc_ms=None, release=True))
L("activityEnd", release=True, need_final=st["need_final"])
dl = now() + 25
while now() < dl:
    poll(); time.sleep(0.002)
    if st["need_final"]:
        if st.get("ack") and now() - st["ack"] >= 3: st["need_final"] = False; L("empty_turn_release"); break
    elif (st.get("gen") and now() - st["end_at"] > 0.7 and now() - st.get("lastmsg", 0) > 0.7) or now() - st["end_at"] > 1.5 + (ends[-1]["final_ms"] or 0) / 1000:
        break
L("done", release_wait_s=round(now() - rel, 2))
over = [e for e in ends if e["final_ms"] is None or e["final_ms"] > 1500]
print(f"SUMMARY file={a.wav.split('/')[-1][:8]} audio_s={len(pcm)/16000:.1f} boundaries={len(ends)} "
      f"final_ms={[e['final_ms'] for e in ends]} tc_ms={[e['tc_ms'] for e in ends]} over_1500ms={len(over)}")
print("TEXT:", " ".join(texts))
if a.json: json.dump(log, open(a.json, "w"))
