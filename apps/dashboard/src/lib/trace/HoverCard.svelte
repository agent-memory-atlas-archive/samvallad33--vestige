<script lang="ts">
	/**
	 * The memory under the pointer, shown beside it: what kind it is and when
	 * it was written, its words, and what a click does. One card for every
	 * field (the organs on TraceStage and the Field), so a memory reads the
	 * same wherever it is hovered.
	 */
	interface Props {
		/** Where the memory is on screen, in px of the viewport. */
		x: number;
		y: number;
		/** Open to the left (the memory is near the right edge). */
		left?: boolean;
		/** CSS colour of the memory's kind. */
		tone: string;
		kicker: string;
		text: string;
		/** An extra line, for example which memory of a pill this is. */
		note?: string | null;
		/** What a click does. */
		action: string;
	}

	let { x, y, left = false, tone, kicker, text, note = null, action }: Props = $props();
</script>

<div class="card" class:left style:left="{x}px" style:top="{y}px" style:--tone={tone} role="tooltip">
	<span class="card-kicker">{kicker}</span>
	<span class="card-text">{text}</span>
	{#if note}
		<span class="card-note">{note}</span>
	{/if}
	<span class="card-open">{action}</span>
</div>

<style>
	.card {
		position: fixed;
		transform: translate(20px, -50%);
		width: max-content;
		max-width: min(22rem, 70vw);
		pointer-events: none;
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		padding: 0.55rem 0.7rem 0.5rem;
		border-radius: 8px;
		background: rgba(4, 6, 12, 0.94);
		border-left: 2px solid var(--tone);
		box-shadow:
			0 0 0 1px color-mix(in srgb, var(--tone) 22%, transparent),
			0 10px 30px rgba(0, 0, 0, 0.6);
		z-index: 70;
		animation: card-in 0.16s cubic-bezier(0.2, 0.8, 0.2, 1) both;
	}
	.card.left {
		transform: translate(calc(-100% - 20px), -50%);
		border-left: none;
		border-right: 2px solid var(--tone);
	}
	@keyframes card-in {
		from {
			opacity: 0;
			margin-left: -6px;
		}
	}
	.card-kicker {
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.6rem;
		letter-spacing: 0.16em;
		text-transform: uppercase;
		color: var(--tone);
	}
	.card-text {
		font-size: 0.82rem;
		line-height: 1.42;
		color: #eef6f4;
	}
	.card-note {
		font-size: 0.68rem;
		color: #9fb3b0;
	}
	.card-open {
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.56rem;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: #5f7c78;
	}
	@media (prefers-reduced-motion: reduce) {
		.card {
			animation: none;
		}
	}
</style>
