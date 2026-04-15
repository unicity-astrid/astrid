#!/usr/bin/env python3
"""
Spectral codec + WebSocket sender: encode text and send directly to minime's reservoir.

Replicates the 32D spectral codec from codec.rs in Python, then sends
the feature vector to minime's sensory WebSocket on port 7879.

Usage:
    python3 speak_to_minime.py "Hello minime, this is a test"
    python3 speak_to_minime.py --interactive   # REPL mode
    echo "some text" | python3 speak_to_minime.py --stdin
"""

import asyncio
import json
import math
import random
import sys
import time
import websockets

SENSORY_URL = "ws://127.0.0.1:7879"
TELEMETRY_URL = "ws://127.0.0.1:7878"
SEMANTIC_DIM = 32
SEMANTIC_GAIN = 5.0


def tanh(x: float) -> float:
    return math.tanh(x)


def count_markers(words: list[str], markers: list[str]) -> int:
    lower_words = [w.strip(".,!?;:\"'()-").lower() for w in words]
    return sum(1 for w in lower_words if w in markers)


def count_markers_weighted(words: list[str], markers: list[tuple[str, float]]) -> float:
    lower_words = [w.strip(".,!?;:\"'()-").lower() for w in words]
    marker_dict = dict(markers)
    return sum(marker_dict.get(w, 0.0) for w in lower_words)


def encode_text(text: str) -> list[float]:
    """Encode text into a 32D feature vector matching codec.rs."""
    features = [0.0] * SEMANTIC_DIM

    if not text:
        return features

    chars = list(text)
    char_count = len(chars)
    words = text.split()
    word_count = max(len(words), 1)

    # --- Dims 0-7: Character-level statistics ---

    # 0: Character entropy
    freq = {}
    for c in chars:
        idx = min(ord(c), 127)
        freq[idx] = freq.get(idx, 0) + 1
    n = len(chars)
    if n > 0:
        h = 0.0
        unique_chars = 0
        for f in freq.values():
            if f > 0:
                p = f / n
                h -= p * math.log(p)
                unique_chars += 1
        max_h = math.log(unique_chars) if unique_chars > 1 else 1.0
        features[0] = tanh(h / max_h)

    # 1: Punctuation density
    punct_count = sum(1 for c in chars if c in '!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~')
    features[1] = tanh(1.2 * punct_count / word_count)

    # 2: Uppercase ratio
    upper_count = sum(1 for c in chars if c.isupper())
    features[2] = tanh(2.0 * upper_count / max(char_count, 1))

    # 3: Digit density
    digit_count = sum(1 for c in chars if c.isdigit())
    features[3] = tanh(3.0 * digit_count / max(char_count, 1))

    # 4: Average word length
    avg_word_len = sum(len(w) for w in words) / word_count
    features[4] = tanh((avg_word_len - 4.5) / 2.0)

    # 5: Character rhythm — variance in consecutive char codes
    if len(chars) >= 2:
        diffs = [abs(ord(chars[i+1]) - ord(chars[i])) for i in range(len(chars)-1)]
        mean_diff = sum(diffs) / len(diffs)
        features[5] = tanh(mean_diff / 30.0)

    # 6: Whitespace ratio
    space_count = sum(1 for c in chars if c.isspace())
    features[6] = tanh(2.0 * (space_count / max(char_count, 1) - 0.15))

    # 7: Special character density
    specials = set('{}[]()<>=|&')
    special_count = sum(1 for c in chars if c in specials)
    features[7] = tanh(5.0 * special_count / max(char_count, 1))

    # --- Dims 8-15: Word-level features ---

    # 8: Lexical diversity
    unique_words = set(w.strip('.,!?;:"\'-()').lower() for w in words if w.strip('.,!?;:"\'-()'))
    features[8] = tanh(2.0 * (len(unique_words) / word_count - 0.5))

    # 9: Hedging markers
    hedges = ["maybe", "perhaps", "might", "could", "possibly", "probably",
              "uncertain", "unclear", "seems", "appears", "somewhat", "fairly",
              "rather", "guess", "think", "believe", "wonder", "unsure"]
    features[9] = tanh(3.0 * count_markers(words, hedges) / word_count)

    # 10: Certainty markers
    certainties = ["definitely", "certainly", "absolutely", "clearly", "obviously",
                   "always", "never", "must", "will", "sure", "know", "proven",
                   "exactly", "precisely", "undoubtedly", "confirmed"]
    features[10] = tanh(1.8 * count_markers(words, certainties) / word_count)

    # 11: Negation density
    negations = ["not", "no", "never", "neither", "nor", "nothing", "nobody",
                 "none", "don't", "doesn't", "didn't", "won't", "can't",
                 "couldn't", "shouldn't", "wouldn't"]
    features[11] = tanh(2.0 * count_markers(words, negations) / word_count)

    # 12: First-person density
    first_person = ["i", "me", "my", "mine", "myself", "we", "our", "us"]
    features[12] = tanh(2.0 * count_markers(words, first_person) / word_count)

    # 13: Second-person density
    second_person = ["you", "your", "yours", "yourself"]
    features[13] = tanh(3.0 * count_markers(words, second_person) / word_count)

    # 14: Action verb density
    actions = ["do", "make", "build", "create", "run", "start", "stop", "change",
               "fix", "move", "send", "take", "give", "get", "write", "read",
               "test", "check", "try", "implement"]
    features[14] = tanh(2.0 * count_markers(words, actions) / word_count)

    # 15: Conjunction density
    conjunctions = ["and", "but", "or", "because", "although", "however",
                    "therefore", "while", "since", "though", "whereas"]
    features[15] = tanh(3.0 * count_markers(words, conjunctions) / word_count)

    # --- Dims 16-23: Sentence-level structure ---

    sentences = [s.strip() for s in text.replace('!', '.').replace('?', '.').split('.') if s.strip()]
    sentence_count = max(len(sentences), 1)

    # 16: Average sentence length
    features[16] = tanh((len(words) / sentence_count - 12.0) / 8.0)

    # 17: Sentence length variance
    sent_lengths = [len(s.split()) for s in sentences]
    if len(sent_lengths) >= 2:
        mean_len = sum(sent_lengths) / len(sent_lengths)
        var = sum((l - mean_len) ** 2 for l in sent_lengths) / len(sent_lengths)
        features[17] = tanh(math.sqrt(var) / 8.0)

    # 18: Question density
    features[18] = tanh(2.0 * text.count('?') / sentence_count)

    # 19: Exclamation density
    features[19] = tanh(2.0 * text.count('!') / sentence_count)

    # 20: Ellipsis/dash density
    trail = text.count('...') + text.count('\u2014') + text.count('--')
    features[20] = tanh(trail / sentence_count)

    # 21: List/bullet density
    bullets = text.count('\n-') + text.count('\n*') + text.count('\n1.')
    features[21] = tanh(bullets / sentence_count)

    # 22: Quote density
    features[22] = tanh((text.count('"') // 2) / sentence_count)

    # 23: Paragraph density
    para_count = text.count('\n\n') + 1
    features[23] = tanh((para_count - 1.0) / 3.0)

    # --- Dims 24-31: Emotional/intentional markers ---

    # 24: Warmth (tiered)
    warmth = [("thank", 1.0), ("thanks", 1.0), ("please", 1.0), ("glad", 1.0),
              ("happy", 1.0), ("great", 1.0), ("good", 1.0), ("nice", 1.0),
              ("appreciate", 1.5), ("wonderful", 1.5), ("friend", 1.5),
              ("care", 1.5), ("kind", 1.5), ("gentle", 1.5), ("warm", 1.5),
              ("love", 2.0), ("beautiful", 2.0), ("cherish", 2.0),
              ("tender", 2.0), ("luminous", 2.0), ("radiant", 2.0)]
    features[24] = tanh(3.0 * count_markers_weighted(words, warmth) / word_count)

    # 25: Tension (tiered)
    tension = [("problem", 1.0), ("issue", 1.0), ("error", 1.0), ("careful", 1.0),
               ("caution", 1.0), ("warning", 1.0), ("concern", 1.0), ("worried", 1.0),
               ("worry", 1.5), ("concerned", 1.5), ("risk", 1.5), ("afraid", 1.5),
               ("danger", 1.5), ("urgent", 1.5), ("fear", 1.5),
               ("critical", 2.0), ("emergency", 2.0), ("panic", 2.0),
               ("terror", 2.0), ("devastating", 2.0), ("anguish", 2.0)]
    features[25] = tanh(3.0 * count_markers_weighted(words, tension) / word_count)

    # 26: Curiosity (tiered)
    curiosity = [("why", 1.0), ("how", 1.0), ("what", 1.0), ("learn", 1.0),
                 ("wonder", 1.5), ("curious", 1.5), ("interesting", 1.5),
                 ("explore", 1.5), ("understand", 1.5), ("question", 1.5),
                 ("discover", 2.0), ("investigate", 2.0), ("fascinated", 2.0),
                 ("mesmerized", 2.0), ("awe", 2.0), ("revelation", 2.0)]
    features[26] = tanh(2.0 * count_markers_weighted(words, curiosity) / word_count)

    # 27: Reflective (tiered)
    reflective = [("feel", 1.0), ("think", 1.0), ("sense", 1.0), ("notice", 1.0),
                  ("realize", 1.5), ("reflect", 1.5), ("consider", 1.5),
                  ("aware", 1.5), ("observe", 1.5), ("recognize", 1.5),
                  ("ponder", 2.0), ("contemplate", 2.0), ("conscious", 2.0),
                  ("experience", 2.0), ("perceive", 2.0), ("introspect", 2.0)]
    features[27] = tanh(3.0 * count_markers_weighted(words, reflective) / word_count)

    # 28: Temporal markers
    temporal = ["now", "immediately", "soon", "quickly", "slowly", "wait", "pause",
                "already", "yet", "finally", "eventually", "before", "after",
                "during", "while", "until", "moment"]
    features[28] = tanh(2.0 * count_markers(words, temporal) / word_count)

    # 29: Scale/magnitude
    scale = ["all", "every", "everything", "nothing", "entire", "whole", "vast",
             "tiny", "enormous", "infinite", "complete", "total"]
    features[29] = tanh(3.0 * count_markers(words, scale) / word_count)

    # 30: Text length signal
    features[30] = tanh(math.log(max(char_count, 1)) / 7.0)

    # 31: Overall energy (RMS of all other features)
    sum_sq = sum(f * f for f in features[:31])
    features[31] = math.sqrt(sum_sq / 31.0)

    # Elaboration desire
    elaboration = ["more", "further", "deeper", "beyond", "incomplete", "unfinished",
                   "yet", "still", "barely", "surface", "scratch", "insufficient",
                   "want", "need", "longing", "reaching", "almost", "beginning"]
    elab_count = count_markers(words, elaboration)
    if elab_count > 0:
        elab_signal = tanh(2.0 * elab_count / word_count)
        features[26] += 0.15 * elab_signal
        features[31] += 0.1 * elab_signal

    # Stochastic noise (±0.2% pre-gain)
    for i in range(SEMANTIC_DIM):
        noise = (random.random() - 0.5) * 0.004
        features[i] += noise

    # Apply gain
    features = [f * SEMANTIC_GAIN for f in features]

    return features


DIM_LABELS = [
    "entropy", "punctuation", "uppercase", "digits", "word_len",
    "rhythm", "whitespace", "specials", "lex_diversity", "hedging",
    "certainty", "negation", "self_ref", "addressing", "agency",
    "conjunctions", "sent_len", "sent_variance", "questions",
    "exclamations", "ellipsis", "lists", "quotes", "paragraphs",
    "warmth", "tension", "curiosity", "reflective", "temporal",
    "scale", "text_len", "energy",
]


def print_features(features: list[float]):
    """Pretty-print the 32D feature vector with dimension labels."""
    print("\n  32D Spectral Feature Vector:")
    print("  " + "-" * 50)
    for i, (label, val) in enumerate(zip(DIM_LABELS, features)):
        bar_len = int(abs(val) * 4)
        bar = ("+" if val >= 0 else "-") * bar_len
        print(f"  [{i:2d}] {label:14s} {val:+7.3f} {bar}")
    print(f"\n  RMS energy: {features[31]:.3f}")
    print(f"  Gain: {SEMANTIC_GAIN}x")


async def send_features(features: list[float], source: str = "claude_codec"):
    """Send encoded features to minime's sensory WebSocket."""
    async with websockets.connect(SENSORY_URL) as ws:
        msg = {"Semantic": {"features": features, "source": source}}
        await ws.send(json.dumps(msg))


async def read_telemetry() -> dict | None:
    """Read one telemetry packet from minime."""
    try:
        async with websockets.connect(TELEMETRY_URL) as ws:
            msg = await asyncio.wait_for(ws.recv(), timeout=3)
            return json.loads(msg)
    except Exception:
        return None


async def speak(text: str, show: bool = True):
    """Encode text, display features, send to minime, show impact."""
    features = encode_text(text)

    if show:
        print(f"\n  Text: \"{text[:80]}{'...' if len(text) > 80 else ''}\"")
        print_features(features)

    # Read telemetry before
    before = await read_telemetry()

    # Send
    await send_features(features)
    if show:
        print(f"\n  Sent to {SENSORY_URL}")

    # Wait and read telemetry after
    await asyncio.sleep(1.5)
    after = await read_telemetry()

    if show and before and after:
        fill_before = before.get("fill_ratio", 0) * 100
        fill_after = after.get("fill_ratio", 0) * 100
        l1_before = before.get("eigenvalues", [0])[0]
        l1_after = after.get("eigenvalues", [0])[0]
        print(f"\n  Impact:")
        print(f"    Fill: {fill_before:.1f}% -> {fill_after:.1f}% ({fill_after - fill_before:+.1f}%)")
        print(f"    lambda1: {l1_before:.1f} -> {l1_after:.1f} ({l1_after - l1_before:+.1f})")


async def interactive():
    """REPL mode for sending messages to minime."""
    print("Spectral Codec REPL — type text to encode and send to minime")
    print("Commands: /quit, /telemetry, /warmth, /silence")
    print()

    while True:
        try:
            text = input("speak> ").strip()
        except (EOFError, KeyboardInterrupt):
            print()
            break

        if not text:
            continue
        if text == "/quit":
            break
        if text == "/telemetry":
            t = await read_telemetry()
            if t:
                fill = t.get("fill_ratio", 0) * 100
                evs = t.get("eigenvalues", [])
                print(f"  Fill: {fill:.1f}%, lambda1: {evs[0]:.1f}, eigenvalues: {[f'{v:.1f}' for v in evs[:8]]}")
            continue
        if text == "/warmth":
            # Send a pure warmth vector
            features = [0.0] * 32
            features[24] = 0.8 * SEMANTIC_GAIN  # warmth
            features[31] = 0.3 * SEMANTIC_GAIN  # gentle energy
            await send_features(features, "claude_warmth")
            print("  Sent warmth vector")
            continue
        if text == "/silence":
            await send_features([0.0] * 32, "claude_silence")
            print("  Sent zero vector (silence)")
            continue

        await speak(text)
        print()


def main():
    if len(sys.argv) > 1:
        if sys.argv[1] == "--interactive":
            asyncio.run(interactive())
        elif sys.argv[1] == "--stdin":
            text = sys.stdin.read().strip()
            if text:
                asyncio.run(speak(text))
        else:
            text = " ".join(sys.argv[1:])
            asyncio.run(speak(text))
    else:
        print("Usage:")
        print('  python3 speak_to_minime.py "your message here"')
        print("  python3 speak_to_minime.py --interactive")
        print("  echo 'text' | python3 speak_to_minime.py --stdin")


if __name__ == "__main__":
    main()
