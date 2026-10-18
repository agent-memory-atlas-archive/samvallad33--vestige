<script lang="ts">
	/**
	 * MemoryFocus — what every organ shows when a memory is clicked.
	 *
	 * The things that matter about one memory, all read live:
	 *   - its own words, type, tags and how strongly it is held right now
	 *     (memory get, memory state: FSRS stability, retrieval and storage
	 *     strength, last access);
	 *   - its proof: the receipt re-verified against the hash-chained log,
	 *     and the exact frame in the log that holds it (receipt get,
	 *     memory_status provenance);
	 *   - its place in the record: every recorded edge that touches it, in
	 *     either direction (ghostlink map, depth 1), each one a way to walk on.
	 * The page can draw those edges on the field through `onlinks`.
	 */
	import { untrack } from 'svelte';
	import { alive } from './alive';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import { edgeColor, typeColor, type VerifyState } from './trace-pass';
	import { headline, isCausalEdge, normalizeReceipt, type ReceiptProof, type TraceLink } from './trace-data';
	import { slowProof } from './slow-proof';

	interface Props {
		id: string;
		/** Go to another memory (a connected one). */
		onpick?: (id: string) => void;
		/** The recorded edges around this memory, for the field to draw. */
		onlinks?: (links: TraceLink[]) => void;
		onclose?: () => void;
		/** The receipt check, for the field's verification sweep and seal. */
		onverify?: (state: VerifyState) => void;
		/** Words that belong to the organ (shown under the memory's own words). */
		children?: import('svelte').Snippet;
		/** The memory itself, once read (type, retention, stability, words). */
		onnode?: (node: FocusNode) => void;
		/** Its neighbours by recorded edge, with their types and words. */
		onneighbours?: (list: FocusNeighbour[]) => void;
		/** Hovering a neighbour in the list (null when the pointer leaves). */
		onhoverneighbour?: (id: string | null) => void;
		/** Hide the "Why it happened, on film" link (the page already is the film). */
		bare?: boolean;
		/** Bump to re-verify the receipt now. */
		proofNonce?: number;
		/**
		 * 'auto' re-verifies the receipt on every pick (default); 'manual' waits
		 * for a click, so a page that is read pick after pick stays instant.
		 */
		proof?: 'auto' | 'manual';
	}
	let { id, onpick, onlinks, onclose, onverify, children, onnode, onneighbours, onhoverneighbour, bare = false, proofNonce = 0, proof = 'auto' }: Props = $props();

	interface FocusNode {
		id: string;
		nodeType?: string;
		content?: string;
		createdAt?: string;
		retentionStrength?: number;
		tags?: string[];
		source?: string;
		stability?: number;
		lastAccessed?: string;
	}
	interface FocusNeighbour {
		id: string;
		label: string;
		type: string;
		edge: string;
		outgoing: boolean;
	}

	let node = $state<FocusNode | null>(null);
	let strength = $state<{ retrieval?: number; storage?: number } | null>(null);
	let receipt = $state<ReceiptProof | null>(null);
	let receiptMs = $state<number | null>(null);
	let receiptError = $state<string | null>(null);
	let frame = $state<{ seq?: number; gateSeq?: number; kindName?: string; op?: string; frameHash?: string } | null>(null);
	let neighbours = $state<FocusNeighbour[] | null>(null);
	let error = $state<string | null>(null);

	/** How long the slow proofs (receipt re-verification, provenance) wait after a pick. */
	const PROOF_DELAY_MS = 450;
	async function load(target: string) {
		node = null;
		strength = null;
		receipt = null;
		receiptMs = null;
		receiptError = null;
		frame = null;
		neighbours = null;
		error = null;
		// Compared with the id this card last loaded, never by reading the prop:
		// a page that clears its pick tears this card down while answers are
		// still on their way, and its prop getter may no longer resolve.
		const stale = () => target !== current;
		api.tool<{ node?: FocusNode }>('memory', { action: 'get', id: target })
			.then((r) => {
				if (stale()) return;
				node = r.node ?? null;
				if (node) onnode?.(node);
			})
			.catch((e) => !stale() && (error = e instanceof Error ? e.message : String(e)));
		api.tool<{ components?: { retrievalStrength?: number; storageStrength?: number } }>('memory', { action: 'state', id: target })
			.then((r) => !stale() && (strength = { retrieval: r.components?.retrievalStrength, storage: r.components?.storageStrength }))
			.catch(() => {});
		api.tool<{ nodes?: { id: string; label?: string; type?: string; isCenter?: boolean }[]; edges?: { source: string; target: string; type: string }[] }>(
			'ghostlink',
			{ mode: 'map', center_id: target, depth: 1, max_nodes: 40 }
		)
			.then((r) => {
				if (stale()) return;
				const byId = new Map((r.nodes ?? []).map((n) => [n.id, n]));
				const list: FocusNeighbour[] = [];
				// The log can record the same edge between two memories more than
				// once (a handle shared twice); each relation is listed once.
				const seen = new Set<string>();
				for (const e of r.edges ?? []) {
					const outgoing = e.source === target;
					const other = outgoing ? e.target : e.source;
					if (other === target) continue;
					const key = `${other}|${e.type}|${outgoing}`;
					if (seen.has(key)) continue;
					seen.add(key);
					const n = byId.get(other);
					list.push({ id: other, label: n?.label ?? other, type: n?.type ?? 'memory', edge: e.type, outgoing });
				}
				neighbours = list;
				onneighbours?.(list);
				onlinks?.((r.edges ?? []).map((e) => ({ source: e.source, target: e.target, type: e.type })));
			})
			.catch(() => !stale() && (neighbours = []));
		// The slow proofs wait a beat: re-verifying the chain holds the store for
		// seconds, and every other read queues behind it. The words and edges
		// land first, and a pick that moves on in the meantime never starts them.
		proofAsked = false;
		if (untrack(() => proof) === 'manual') {
			onverify?.('idle');
			return;
		}
		onverify?.('running');
		setTimeout(() => !stale() && runProofs(target, stale), PROOF_DELAY_MS);
	}

	let proofAsked = $state(false);
	/** The receipt and the frame that wrote it, through the one slow-proof lane. */
	function runProofs(target: string, stale: () => boolean) {
		proofAsked = true;
		slowProof(stale, () =>
			Promise.allSettled([
				api.tool<{ origin?: { frame?: typeof frame } }>('memory_status', { view: 'provenance', memoryId: target })
					.then((r) => !stale() && (frame = r.origin?.frame ?? null)),
				verifyReceipt(target, stale)
			])
		);
	}

	/** Re-verify the memory's receipt against the hash-chained log, right now. */
	function verifyReceipt(target: string, stale: () => boolean): Promise<unknown> {
		receipt = null;
		receiptMs = null;
		receiptError = null;
		const t0 = performance.now();
		onverify?.('running');
		return api.tool('receipt', { action: 'get', receipt_id: target })
			.then((r) => {
				if (stale()) return;
				receipt = normalizeReceipt(r);
				receiptMs = Math.round(performance.now() - t0);
				onverify?.(receipt.chainValid ? 'ok' : 'fail');
			})
			.catch((e) => {
				if (stale()) return;
				receiptError = e instanceof Error ? e.message : String(e);
				onverify?.('fail');
			});
	}

	// Load when the id changes, and only then: the callbacks reach into the
	// page, and nothing they read may re-run this load.
	let current: string | null = null;
	$effect(() => {
		const target = id;
		current = target;
		untrack(() => void load(target));
		return () => {
			if (current === target) current = null;
		};
	});
	// The page can ask for the proof again (a click on the memory's form or ring).
	let lastNonce = 0;
	$effect(() => {
		const n = proofNonce;
		if (n === lastNonce) return;
		lastNonce = n;
		const target = untrack(() => id);
		untrack(() => runProofs(target, () => target !== current));
	});

	const EDGE_WORDS: Record<string, [string, string]> = {
		derived_from: ['derived from', 'gave rise to'],
		evidence_of: ['is evidence of', 'is backed by'],
		closes: ['closes', 'is closed by'],
		closed_by: ['was closed by', 'closed'],
		supersedes: ['supersedes', 'was superseded by'],
		corrects: ['corrects', 'was corrected by'],
		contradicts: ['contradicts', 'is contradicted by'],
		touched: ['shares a handle with', 'shares a handle with']
	};
	const relation = (n: FocusNeighbour) => (EDGE_WORDS[n.edge] ?? [n.edge, `${n.edge} (in)`])[n.outgoing ? 0 : 1];
	const causalFirst = $derived(
		[...(neighbours ?? [])].sort((a, b) => Number(isCausalEdge(b.edge)) - Number(isCausalEdge(a.edge)))
	);

	const rgb = ([r, g, b]: [number, number, number]) => `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	const pct = (v?: number) => (typeof v === 'number' ? `${Math.round(v * 100)}%` : '?');
	function when(iso?: string): string {
		if (!iso) return '';
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
	}
	const short = (h?: string) => (h ? `${h.slice(0, 12)}…` : '—');
</script>

{#if error}
	<p class="tr-proof-line tr-err">{error}</p>
{:else if !node}
	<p class="tr-proof-line">Reading the memory by its exact id…</p>
{:else}
	<p class="tr-kicker" style:color={rgb(typeColor(node.nodeType ?? ''))} use:alive={{ text: `${node.nodeType ?? 'memory'} · ${when(node.createdAt)} · ${pct(node.retentionStrength)} retained` }}></p>
	<p class="tr-text focus-text" use:alive={{ text: node.content ?? '' }}></p>
	<p class="tr-id">{node.id}</p>
	{#if node.tags?.length}
		<p class="tr-hint">
			{#each node.tags as tag (tag)}
				<a class="tr-chip" href={osHref('/trace', { q: tag })}>#{tag}</a>
			{/each}
		</p>
	{/if}
	<p class="tr-note">
		{#if typeof node.stability === 'number'}held for about {Math.round(node.stability)} days before it fades · {/if}
		{#if strength}retrieval {pct(strength.retrieval)}, storage {pct(strength.storage)} · {/if}
		{#if node.lastAccessed}last read {when(node.lastAccessed)}{/if}
		{#if node.source}<br />source: {node.source}{/if}
	</p>
{/if}

{@render children?.()}

<div class="tr-block">
	<p class="tr-proof-kicker" use:alive={{ text: 'Proof' }}></p>
	{#if receiptError}
		<p class="tr-proof-line tr-err">{receiptError}</p>
	{:else if !receipt && proof === 'manual' && !proofAsked}
		<p class="tr-proof-line">Its receipt is on the hash-chained log.</p>
		<div class="tr-actions">
			<button type="button" class="tr-act alt" onclick={() => runProofs(id, () => id !== current)}>Re-verify its receipt now</button>
		</div>
	{:else if !receipt}
		<p class="tr-proof-line">Re-verifying the hash-chained log…</p>
	{:else}
		<p class="tr-proof-line">
			{#if receipt.chainValid}<span class="tr-ok">chain valid</span>{:else}<span class="tr-err">chain not valid</span>{/if}
			· {receipt.framesVerified?.toLocaleString() ?? '?'} frames · {receipt.segmentsVerified ?? '?'} segments
		</p>
		<p class="tr-note">receipt {receipt.receiptId ?? '—'} · re-verified just now in {receiptMs} ms</p>
	{/if}
	{#if frame}
		<p class="tr-note">
			written at frame {frame.seq?.toLocaleString() ?? '?'} ({frame.kindName ?? 'write'}{frame.op ? `, ${frame.op}` : ''}) · gate {frame.gateSeq?.toLocaleString() ?? '?'} · frame hash {short(frame.frameHash)}
		</p>
	{/if}
</div>

<div class="tr-block">
	<p class="tr-proof-kicker" use:alive={{ text: 'Connected by recorded edges' }}></p>
	{#if neighbours === null}
		<p class="tr-proof-line">Reading its edges…</p>
	{:else if neighbours.length === 0}
		<p class="tr-proof-line">No recorded edge touches this memory yet.</p>
	{:else}
		{#each causalFirst.slice(0, 12) as n (n.id + n.edge + n.outgoing)}
			<button
				type="button"
				class="tr-item"
				data-trace-id={n.id}
				onclick={() => onpick?.(n.id)}
				onpointerenter={() => onhoverneighbour?.(n.id)}
				onpointerleave={() => onhoverneighbour?.(null)}
			>
				<span class="tr-edge" style:color={rgb(edgeColor(n.edge))}>{relation(n)}</span>
				<span>{headline(n.label, 110)}</span>
			</button>
		{/each}
		{#if causalFirst.length > 12}<p class="tr-note">and {causalFirst.length - 12} more</p>{/if}
	{/if}
</div>

<div class="tr-actions">
	{#if !bare}<a class="tr-act alt" href={osHref('/trace', { q: id })}>Why it happened, on film →</a>{/if}
	{#if onclose}<button type="button" class="tr-act quiet" onclick={() => onclose?.()}>Back</button>{/if}
</div>

<style>
	.focus-text {
		max-height: 13rem;
		overflow-y: auto;
		scrollbar-width: none;
	}
	a.tr-act {
		text-decoration: none;
	}
</style>
