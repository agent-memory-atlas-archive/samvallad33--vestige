<script lang="ts">
	/**
	 * Brain print panel — structure-only signature of the live store.
	 * Print id, trait chips, permalink. Zero memory text.
	 */
	import type { BrainPrint, BrainTrait } from '$lib/observatory/brain-print';

	interface Props {
		print: BrainPrint | null;
		activePrintId: string | null;
		printing: boolean;
		error: string | null;
		copied: boolean;
		disabled: boolean;
		onprint: () => void;
		oncopy: () => void;
	}

	let {
		print,
		activePrintId,
		printing,
		error,
		copied,
		disabled,
		onprint,
		oncopy
	}: Props = $props();

	const traits: BrainTrait[] = $derived(print?.traits ?? []);
</script>

<div class="obs-print">
	<button
		class="obs-print-btn"
		type="button"
		onclick={onprint}
		disabled={disabled || printing}
		aria-busy={printing}
	>
		{printing ? 'Reading shape…' : print ? 'Recompute print' : 'Brain print'}
	</button>
	{#if error}
		<span class="obs-print-error" role="status">{error}</span>
	{/if}
	{#if activePrintId}
		<div class="obs-print-card" aria-live="polite">
			<span class="obs-print-kicker">structure only · zero memory text</span>
			<code class="obs-print-id">{activePrintId}</code>
			{#if traits.length}
				<ul class="obs-traits">
					{#each traits as trait (trait.id)}
						<li class="obs-chip">{trait.label}</li>
					{/each}
				</ul>
			{/if}
			<button class="obs-print-copy" type="button" onclick={oncopy}>
				{copied ? 'Permalink copied' : 'Copy permalink'}
			</button>
		</div>
	{/if}
</div>

<style>
	.obs-print {
		display: flex;
		flex-direction: column;
		gap: 0.55rem;
		margin-top: 0.15rem;
	}
	/* Bare words over the field, like every organ: no boxes. */
	.obs-print-btn {
		align-self: flex-start;
		padding: 0.2rem 0;
		border: none;
		border-bottom: 1px solid rgba(127, 243, 230, 0.5);
		background: none;
		color: #7ff3e6;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.7rem;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		cursor: pointer;
		text-shadow: 0 0 22px rgba(2, 3, 7, 0.95), 0 1px 2px rgba(2, 3, 7, 0.9);
	}
	.obs-print-btn:hover:not(:disabled) {
		color: #ffffff;
	}
	.obs-print-btn:focus-visible {
		outline: 2px solid rgba(34, 199, 222, 0.7);
		outline-offset: 2px;
	}
	.obs-print-btn:disabled {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.obs-print-card {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
		padding: 0.2rem 0 0.2rem 0.8rem;
		border-left: 1px solid rgba(127, 243, 230, 0.2);
		text-shadow: 0 0 22px rgba(2, 3, 7, 0.95), 0 1px 2px rgba(2, 3, 7, 0.9);
	}
	.obs-print-kicker {
		font-size: 0.62rem;
		letter-spacing: 0.16em;
		text-transform: uppercase;
		color: rgba(140, 199, 180, 0.62);
	}
	.obs-print-id {
		font-size: 0.98rem;
		font-variant-numeric: tabular-nums;
		letter-spacing: 0.08em;
		color: #7ff3e6;
		text-shadow: 0 0 16px rgba(34, 199, 222, 0.45);
	}
	.obs-traits {
		display: flex;
		flex-wrap: wrap;
		gap: 0.35rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.obs-chip {
		padding: 0;
		color: #9fdcc8;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.64rem;
		letter-spacing: 0.08em;
	}
	.obs-print-copy {
		align-self: flex-start;
		margin-top: 0.1rem;
		padding: 0;
		border: 0;
		background: transparent;
		color: #8cc7db;
		font-size: 0.76rem;
		letter-spacing: 0.02em;
		cursor: pointer;
	}
	.obs-print-copy:hover {
		text-decoration: underline;
		color: #eafffb;
	}
	.obs-print-error {
		color: #ff6a5e;
		font-size: 0.72rem;
		max-width: 22rem;
	}
</style>
