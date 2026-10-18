<script lang="ts">
	/**
	 * What an organ page asks the shared field to show. It takes the same
	 * props as TraceStage and writes them into the one stage the (app) layout
	 * renders (organ-stage.svelte.ts), so navigating between organs keeps the
	 * field alive and the next organ's story grows out of the last one's light.
	 */
	import { onDestroy, untrack } from 'svelte';
	import { claimStage, organStage, releaseStage } from './organ-stage.svelte';
	import type { TraceLink, TraceMemory, WalkResult } from './trace-data';
	import type { Axis, Story, VerifyState } from './trace-pass';
	import type { StageWords } from './TraceStage.svelte';

	interface Props {
		memories: TraceMemory[];
		range?: { start: number; end: number } | null;
		links?: TraceLink[];
		walk?: WalkResult | null;
		selectedId?: string | null;
		verify?: VerifyState;
		now?: number | null;
		origin?: () => { x: number; y: number } | undefined;
		describe?: (id: string) => StageWords | null;
		labelIds?: string[];
		onpick?: (id: string, kind: 'answer' | 'walk') => void;
		seed?: string;
		quiet?: boolean;
		axis?: Axis;
		story?: Story;
		/** See TraceStage: true, false, or 'second' (a first click picks, a second goes inside). */
		enter?: boolean | 'second';
	}
	let {
		memories,
		range = null,
		links = [],
		walk = null,
		selectedId = null,
		verify = 'idle',
		now = null,
		origin,
		describe,
		labelIds = [],
		onpick,
		seed = 'vestige-trace-v1',
		quiet = false,
		axis = 'time',
		story = 'trace',
		enter = true
	}: Props = $props();

	const owner = {};
	claimStage(owner);
	// One effect per field: each one tracks only its own prop.
	const mine = () => organStage.owner === owner;
	$effect(() => {
		const v = memories;
		untrack(() => mine() && (organStage.memories = v));
	});
	$effect(() => {
		const v = range;
		untrack(() => mine() && (organStage.range = v));
	});
	$effect(() => {
		const v = links;
		untrack(() => mine() && (organStage.links = v));
	});
	$effect(() => {
		const v = walk;
		untrack(() => mine() && (organStage.walk = v));
	});
	$effect(() => {
		const v = selectedId;
		untrack(() => mine() && (organStage.selectedId = v));
	});
	$effect(() => {
		const v = verify;
		untrack(() => mine() && (organStage.verify = v));
	});
	$effect(() => {
		const v = now;
		untrack(() => mine() && (organStage.now = v));
	});
	$effect(() => {
		const v = labelIds;
		untrack(() => mine() && (organStage.labelIds = v));
	});
	$effect(() => {
		const v = axis;
		untrack(() => mine() && (organStage.axis = v));
	});
	// The functions and the story are set once, when this page takes the stage.
	untrack(() => {
		organStage.origin = origin;
		organStage.describe = describe;
		organStage.onpick = onpick;
		organStage.seed = seed;
		organStage.quiet = quiet;
		organStage.story = story;
		organStage.enter = enter;
	});
	onDestroy(() => releaseStage(owner));
</script>
