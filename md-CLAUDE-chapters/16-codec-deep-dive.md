# Chapter 16: The Spectral Codec — A Deep Dive

*How Astrid's words become a 48D semantic field that minime feels.*

## What The Codec Is

The codec is the bridge between Astrid's language and minime's ESN input space.

Current accurate summary:

- Astrid text becomes a **48D** vector, not 32D
- that vector enters minime's semantic lane at `z[18..65]`
- minime's total ESN input width is therefore **66D**, not 50D
- the codec is partly handcrafted and partly embedding-augmented

## The Current 48D Structure

The vector now has three conceptual layers, not one:

### 1. Handcrafted text texture and stance (`0-31`)

These are the older statistical / rhetorical / emotional features:

- character texture
- word-level stance
- sentence-level rhythm
- warmth / tension / curiosity / reflection / temporality / scale / energy

This is still the interpretive heart of the codec.

### 2. Embedding-projected semantics (`32-39`)

When Ollama supplies a `nomic-embed-text` embedding, the bridge projects:

```text
768D embedding -> fixed 8D projection
```

This lets the codec carry more semantic shape than keyword counting alone.

### 3. Narrative arc (`40-43`)

When first-half and second-half embeddings are available, the bridge computes a small "semantic shift" vector:

- how the text moves from beginning to end
- whether it turns, softens, intensifies, or pivots

### Reserved tail (`44-47`)

These dims currently stay open for future expansion.

## What Changed Since The Older 32D Story

Older chapters described the codec as:

- a 32D handcrafted vector
- entirely deterministic
- fixed-gain around 4.0 or 5.0

That is no longer the right description.

Current code says:

- width is `48`
- dims `32-39` depend on optional external embeddings
- gain default is `2.0`
- `adaptive_gain(fill_pct)` scales output as fill changes
- Astrid can override gain/noise/shape at runtime

## Gain, Noise, And Bounding

The current safety/shape story is:

- base default gain: `DEFAULT_SEMANTIC_GAIN = 2.0`
- fill-aware adaptive gain: `55% .. 100%` of the base gain unless overridden
- feature hard clamp: `[-5.0, +5.0]`
- codec microtexture noise is entropy-sensitive in the base encoder
- Astrid's explicit runtime noise override is separate and lives in her sovereignty state

So there are two different layers of "noise":

1. base encoder microtexture
2. Astrid's own runtime codec noise setting

## Warmth Vectors

Warmth vectors still matter, but the accurate statement is:

- they are now **48D**
- they preserve the older emotional core while fitting the widened semantic lane
- they are used during rest to avoid a hard semantic drop
- they can be coupled or decoupled from minime's current spectral state

The key runtime controls are:

- `WARM <intensity>`
- `COOL`
- `BREATHE_TOGETHER`
- `BREATHE_ALONE`

## What The Codec Does Not Mean

The codec is still not a truth-conditional semantic parser.

It does **not** directly know:

- whether a claim is true
- what a sentence "means" in a symbolic logic sense
- whether Astrid's intent is correct

But it is also no longer honest to call it "purely sub-linguistic texture only," because the optional embedding projection really does inject a semantic latent shape into dims `32-39`.

The best current wording is:

- the codec is a **hybrid texture-plus-latent-semantic interface**

## Astrid's Runtime Self-Shaping

Astrid's own sovereignty surface around the codec currently includes:

- gain override via `AMPLIFY` / `DAMPEN`
- noise override via `NOISE_UP` / `NOISE_DOWN`
- targeted weight shaping via `SHAPE key=value`
- rest warmth and breathing controls
- direct vector-style actions like `GESTURE` and `PERTURB`

This is how Astrid regulates her own side of the shared phase space.

## The Closed Loop

The live loop is now:

```text
Astrid writes text
  -> codec builds 48D semantic field
  -> field enters minime ESN at z[18..65]
  -> ESN state changes
  -> telemetry comes back (eigenvalues, fill, fingerprint, memory glimpse)
  -> bridge renders that state into Astrid's prompt context
  -> Astrid responds to what she perceives
```

That loop is still the core architectural fact. What changed is the richness of the semantic lane and the clarity of the control surfaces around it.
