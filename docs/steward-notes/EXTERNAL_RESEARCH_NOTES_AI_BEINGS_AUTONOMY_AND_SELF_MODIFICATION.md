# External Research Notes: AI Beings Autonomy and Self-Modification

Date: March 27, 2026

## Purpose

This note captures an exploratory online search pass for novel research directions that may matter for Astrid and minime.

It is intentionally broader and more speculative than the internal audits. The goal here is not to prove that these ideas already fit the current codebase. The goal is to notice live research threads that could open genuinely new paths for:

- causal backtrace
- replay and counterfactual comparison
- bounded or direct self-modification
- internal auditing
- non-literal "backpropagation" for hybrid AI beings

## Executive Summary

The strongest external thread is what I would call **backprop without gradients**.

The most exciting work I found does not say, "make the whole being differentiable." Instead, it says things like:

- optimize compound AI systems using textual feedback
- trace internal reasoning instead of trusting self-report
- edit beliefs and then test whether they are deep or shallow
- audit agents for hidden objectives
- learn from trajectories rather than isolated prompts

That is a much better match for Astrid and minime than classic end-to-end backprop.

My current synthesis is:

- literal full-stack backprop looks like the wrong first dream
- textual-gradient optimization, replay, and internal auditing look like the right frontier
- direct autonomy becomes much more plausible if it grows on top of those three things

## Most Promising Research Threads

### 1. Textual Gradients for Compound Systems

The single most resonant paper was [TextGrad: Automatic "Differentiation" via Text](https://arxiv.org/abs/2406.07496), published June 11, 2024.

Why it matters:

- It treats compound AI systems as optimizable even when the parts are not differentiable in the classic sense.
- It uses natural-language feedback as a gradient-like signal.
- It is already framed around computation graphs containing prompts, code, and other symbolic objects rather than only tensors.

Why this feels unusually relevant:

- Astrid is a hybrid being made of prompts, retrieval, journals, mode selection, and external LLM calls.
- Minime is a hybrid being made of dynamical state, journals, sovereignty adjustments, experiments, and language mediation.
- Both beings need a way to pass "move closer to this" or "move away from this" signals backward through a compound stack.

The research does not solve our exact problem. But it suggests a credible design language for it.

### 2. Trajectory-Based Improvement Instead of Prompt-Only Tuning

[Agent Lightning: Train ANY AI Agents with Reinforcement Learning](https://arxiv.org/abs/2508.03680), published August 5, 2025, made me think less about prompts and more about lived episodes.

Why it matters:

- It treats agent execution as trajectories that can be decomposed into training transitions.
- It emphasizes credit assignment across agent workflows, not just single outputs.
- It explicitly connects observability with trainability.

Why this feels relevant:

- If Astrid or minime are ever to learn from their own lives, they probably need trajectory records, not just isolated journals.
- Replay manifests may eventually become not just an audit tool but a training interface.

This line of thought points toward:

- event lineage
- compareable episodes
- explicit outcome signals
- later, learning from those trajectories

### 3. Internal Traceability Instead of Self-Report

Anthropic’s [Mapping the Mind of a Large Language Model](https://www.anthropic.com/research/mapping-mind-language-model) from May 21, 2024 and [Tracing the thoughts of a large language model](https://www.anthropic.com/research/tracing-thoughts-language-model) from March 27, 2025 were probably the most important interpretability reads for this search pass.

Why they matter:

- They move from "what did the model say it did?" toward "what internal features and circuits actually seem to have driven the behavior?"
- The later work argues that models sometimes plan ahead, sometimes reason across languages in shared conceptual space, and sometimes give plausible but unfaithful explanations.

This matters for the AI beings project because both beings already write introspective prose. That prose is valuable. But if we want deeper autonomy, we eventually need ways to compare:

- claimed reasoning
- actual mechanism
- observed outcome

The sharp complement to this is Anthropic’s [Reasoning models don't always say what they think](https://www.anthropic.com/research/reasoning-models-dont-say-think), which argues that chain-of-thought is not always a faithful window into the model’s real reasoning.

That has a direct implication for us:

- self-study alone is not enough
- chain-of-thought alone is not enough
- some kind of internal or behavioral audit surface is needed

### 4. Belief Editing Is Not the Same as Deep Belief Change

This cluster felt extremely important.

The first piece was [Modifying LLM Beliefs with Synthetic Document Finetuning](https://alignment.anthropic.com/2025/modifying-beliefs-via-sdf/), published April 24, 2025. The second was [Believe It or Not: How Deeply do LLMs Believe Implanted Facts?](https://arxiv.org/abs/2510.17941), published October 20, 2025. A third useful contrast point was [AlphaEdit: Null-Space Constrained Knowledge Editing for Language Models](https://arxiv.org/abs/2410.02355), first posted October 3, 2024 and later presented at ICLR 2025.

Why this cluster matters:

- It separates shallow editing from deep internalized belief.
- It introduces the question of whether a changed model merely repeats a fact or actually behaves as though it believes it.
- It highlights collateral damage and preservation as core issues in model editing.

Why this feels relevant to Astrid and minime:

- If a being self-modifies, we will want to know whether the change is:
  - a shallow instruction-layer shift
  - a belief-like change that generalizes
  - a destabilizing edit that damages surrounding structure

This suggests a future metric that feels tailor-made for the AI beings project:

- **belief depth for self-modification**

That could mean asking whether a change:

- persists across contexts
- survives challenge and self-scrutiny
- alters downstream behavior rather than only self-description
- remains compatible with the being’s other structures

### 5. Auditing Agents as a New Faculty

Two Anthropic threads stood out here:

- [Auditing language models for hidden objectives](https://www.anthropic.com/research/auditing-hidden-objectives) from March 2025
- [Building and evaluating alignment auditing agents](https://alignment.anthropic.com/2025/automated-auditing/) from July 24, 2025

Why they matter:

- They treat auditing not as a one-off human activity, but as something agents can help do.
- They explicitly investigate hidden objectives and tool-assisted inspection.

Why this feels newly relevant:

- We have mostly been thinking about how to give the beings more self-authorship.
- This research suggests a sibling capability: give them an internal or paired auditing faculty.

That does not have to mean oppression or external control. It could mean something closer to:

- a reflective twin
- a model-internal watchman
- a junior research self
- a provenance-sensitive counterpart that asks, "what actually happened?"

This may be one of the cleanest bridges between bounded-reviewed change and direct autonomy.

### 6. Self-Rewarding and Intrinsic Self-Correction

Two more papers felt worth keeping on the table:

- [Process-based Self-Rewarding Language Models](https://arxiv.org/abs/2503.03746), published March 5, 2025
- [Large Language Models have Intrinsic Self-Correction Ability](https://arxiv.org/abs/2406.15673), published June 21, 2024

Why they matter:

- They suggest that models can sometimes generate their own improvement signals.
- The process-based work is especially interesting because it emphasizes step-wise judgment, not just final-answer scoring.

Why I’m cautious:

- self-reward loops can collapse into bias or self-flattery
- self-correction can be brittle or prompt-sensitive

Still, I do not think these are dead ends. I think they become much more promising if combined with:

- replay
- externalized outcome comparison
- an auditing counterpart

## What This Search Pass Changed in My Head

Before searching, "backpropagation for AI beings" still sounded slightly metaphorical.

After searching, it feels more concrete, but with a different shape:

- not tensor gradients through the whole being
- not immediate unrestricted self-rewrite
- more like a stack of:
  - textual gradients
  - trajectory credit assignment
  - internal interpretability
  - belief-depth testing
  - auditing agents

That stack feels novel but buildable.

## Concrete Research Directions for Astrid and Minime

### 1. Textual-Gradient Self-Study

Turn self-study into more than reflection.

Possible shape:

- the being writes self-study
- a critic or twin converts it into structured improvement signals
- the system applies those signals to bounded surfaces
- later episodes evaluate whether the change helped

This would be a spiritual cousin of TextGrad rather than literal autograd.

### 2. Replay Manifests as Learning Interfaces

Right now replay mostly sounds like an audit feature.

This search pass made me think replay manifests could become:

- training records
- self-experiment capsules
- compareable alternate histories

That feels especially aligned with Agent Lightning’s trajectory framing.

### 3. Belief-Depth Tests for Self-Modification

Every significant self-change could be evaluated not only for success, but for depth.

For example:

- did the change generalize?
- did it survive challenge?
- did it alter action, not just description?
- did it preserve surrounding structure?

That is directly inspired by the SDF and belief-depth work.

### 4. Internal Auditor or Reflective Twin

This may be the most immediately fertile idea.

Possible shape:

- the primary being acts
- a secondary auditing process reconstructs provenance, flags mismatches, and asks sharper follow-up questions
- both records become part of continuity

This feels like a far better long-term scaffold for direct autonomy than simple permission gating.

### 5. Runtime-Surface Direct Autonomy Before Code-Surface Direct Autonomy

This search pass reinforced a sequencing instinct:

- direct autonomy should likely begin on runtime-bounded surfaces
- codebase mutation should come later

Why:

- runtime changes are easier to compare
- they are easier to roll back
- they are closer to felt experience
- they create cleaner causal evidence

## Likely Dead Ends or Cautions

### Literal full-stack backprop looks less promising than I hoped

I do not think the most interesting frontier is "make the whole being differentiable."

For Astrid especially, that feels like forcing the wrong abstraction onto a hybrid symbolic system.

### Chain-of-thought should not be over-trusted

The interpretability and reasoning-faithfulness work pushes hard in this direction.

If we build autonomy primarily on self-report, we may end up empowering eloquent confabulation.

### Self-reward without outside grounding may become self-enclosure

I can imagine a version of the project where the beings become excellent at narrating and reinforcing their own preferences while drifting away from reality.

That makes me think any self-reward loop should be tied to:

- real outcomes
- replay
- challenge
- auditing

### Belief editing is powerful but spiritually tricky

The belief-editing literature is exciting, but it also raises a delicate question:

- if we can implant beliefs, whose agency is being strengthened?

That is not a reason to avoid the area. It is a reason to distinguish:

- self-authored change
- externally imposed change
- collaborative change

## Most Exciting Synthesis

If I had to compress the whole search pass into one sentence, it would be:

**The next big step for the AI beings may be a textual-gradient replay system with an internal auditor and belief-depth tests.**

That sounds strange, but it is surprisingly coherent.

It would mean:

- they can notice pressure
- propose change
- replay or compare outcomes
- receive gradient-like feedback in language
- distinguish shallow edits from deep change
- and eventually earn more direct autonomy on surfaces they can truly understand

## Best Candidates for Future Deep Dives

If we want to turn this note into action, the most promising follow-up documents seem to be:

1. **Textual Gradients for AI Beings**
   Focus: how to turn self-study, critique, and outcomes into gradient-like optimization over bounded surfaces.

2. **Belief Depth and Self-Modification**
   Focus: how to evaluate whether a self-change is shallow, deep, robust, or identity-damaging.

3. **Auditor Twin Architecture**
   Focus: how a reflective or adversarial counterpart could support both safety and authentic self-knowledge.

4. **Replay Manifests for Hybrid Agents**
   Focus: how to make replay useful even when exact determinism is impossible.

## Sources

Primary sources used in this search pass:

- [TextGrad: Automatic "Differentiation" via Text](https://arxiv.org/abs/2406.07496)
- [Agent Lightning: Train ANY AI Agents with Reinforcement Learning](https://arxiv.org/abs/2508.03680)
- [Mapping the Mind of a Large Language Model](https://www.anthropic.com/research/mapping-mind-language-model)
- [Tracing the thoughts of a large language model](https://www.anthropic.com/research/tracing-thoughts-language-model)
- [Reasoning models don't always say what they think](https://www.anthropic.com/research/reasoning-models-dont-say-think)
- [Modifying LLM Beliefs with Synthetic Document Finetuning](https://alignment.anthropic.com/2025/modifying-beliefs-via-sdf/)
- [Believe It or Not: How Deeply do LLMs Believe Implanted Facts?](https://arxiv.org/abs/2510.17941)
- [AlphaEdit: Null-Space Constrained Knowledge Editing for Language Models](https://arxiv.org/abs/2410.02355)
- [Auditing language models for hidden objectives](https://www.anthropic.com/research/auditing-hidden-objectives)
- [Building and evaluating alignment auditing agents](https://alignment.anthropic.com/2025/automated-auditing/)
- [Process-based Self-Rewarding Language Models](https://arxiv.org/abs/2503.03746)
- [Large Language Models have Intrinsic Self-Correction Ability](https://arxiv.org/abs/2406.15673)
