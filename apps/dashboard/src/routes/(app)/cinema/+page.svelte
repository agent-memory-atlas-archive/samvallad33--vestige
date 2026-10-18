<script lang="ts">
	/**
	 * Memory Cinema — its own organ again.
	 *
	 * MEMORY CINEMA IS PROTECTED: this route only feeds the finished component
	 * the same bounded graph the Witness page gives it (48 nodes, depth 2, most
	 * connected first) and presses its own launch button once on arrival. The
	 * component itself is unchanged. Closing the flythrough leaves its launch
	 * button here for a replay.
	 */
	import { onMount, tick } from 'svelte';
	import { page } from '$app/stores';
	import MemoryCinema from '$components/MemoryCinema.svelte';
	import { api } from '$lib/stores/api';
	import { markVeilReadyAfterPaint } from '$lib/trace/handoff';
	import type { GraphResponse } from '$types';

	let graph = $state<GraphResponse | null>(null);
	let error = $state<string | null>(null);
	let host = $state<HTMLDivElement | null>(null);

	onMount(async () => {
		try {
			graph = await api.graph({ max_nodes: 48, depth: 2, sort: 'connected' });
		} catch (cause) {
			error = cause instanceof Error ? cause.message : String(cause);
			markVeilReadyAfterPaint();
			return;
		}
		// ?launch=0 keeps the launch button instead of starting the flythrough.
		if ($page.url.searchParams.get('launch') === '0') {
			markVeilReadyAfterPaint();
			return;
		}
		await tick();
		host?.querySelector<HTMLButtonElement>('.cinema-launch')?.click();
		// Arriving from the Palace: the veil lifts once the flythrough has drawn.
		markVeilReadyAfterPaint(3);
	});
</script>

<svelte:head>
	<title>Memory Cinema · Vestige</title>
</svelte:head>

<div class="cinema-room">
	<div class="cinema-door" bind:this={host}>
		<p class="eyebrow">MEMORY CINEMA</p>
		<h1>Fly through your own memory.</h1>
		{#if graph}
			<MemoryCinema nodes={graph.nodes} edges={graph.edges} centerId={graph.center_id} />
		{:else if error}
			<p class="note">The memory graph could not load: {error}</p>
		{:else}
			<p class="note">Loading your memory graph…</p>
		{/if}
	</div>
</div>

<style>
	.cinema-room {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		background: radial-gradient(ellipse at 50% 60%, #0b1220 0%, #05060a 70%);
		color: #dfe7ef;
	}
	.cinema-door {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.9rem;
		text-align: center;
	}
	.eyebrow {
		font-family: 'SF Mono', ui-monospace, Menlo, monospace;
		font-size: 0.72rem;
		letter-spacing: 0.3em;
		color: #5dcaa5;
		text-shadow: 0 0 12px rgba(93, 202, 165, 0.6);
	}
	/* The film's own light on its title: warm projector gold over cool cyan. */
	h1 {
		font-size: 1.6rem;
		font-weight: 500;
		margin: 0 0 0.5rem;
		color: #fff6e4;
		text-shadow:
			0 0 18px rgba(255, 200, 120, 0.5),
			0 0 44px rgba(110, 220, 255, 0.3);
	}
	.note {
		font-size: 0.85rem;
		color: #7c8a97;
	}
</style>
