<script lang="ts">
	// Mobile navigation is now the ONE registry-driven os-mobilebar in the root
	// +layout.svelte (which also handles ?frame=/?capture= hiding). The old
	// MobileNav is removed to avoid a duplicate mobile bar + a capture-mode leak.
	import TraceStage from '$lib/trace/TraceStage.svelte';
	import { organStage } from '$lib/trace/organ-stage.svelte';

	let { children } = $props();
</script>

<!--
	Full-bleed positioned host for every organ route. The (app) routes render
	full-screen WebGPU canvases with `absolute inset-0` / `h-full`, which resolve
	against the nearest positioned ancestor with a real height box. Without this
	wrapper the parent is `display:contents` (zero-height), so `h-full` collapses
	to 0px and the canvas renders black (the /graph regression). `100dvh` +
	`relative` gives every route a stable full-viewport containing block.
-->
<div class="relative h-[100dvh] w-full overflow-hidden">
	<!-- One field for every organ built on Trace (lib/trace/organ-stage):
	     it stays alive across their pages, so one organ's light becomes the next. -->
	{#if organStage.owner}
		<TraceStage
			memories={organStage.memories}
			range={organStage.range}
			links={organStage.links}
			walk={organStage.walk}
			selectedId={organStage.selectedId}
			verify={organStage.verify}
			now={organStage.now}
			origin={organStage.origin}
			describe={organStage.describe}
			labelIds={organStage.labelIds}
			onpick={organStage.onpick}
			seed={organStage.seed}
			quiet={organStage.quiet}
			axis={organStage.axis}
			story={organStage.story}
			enter={organStage.enter}
		/>
	{/if}
	{@render children()}
</div>
