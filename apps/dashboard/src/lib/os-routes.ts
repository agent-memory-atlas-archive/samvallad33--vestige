// ─────────────────────────────────────────────────────────────────────────────
// os-routes.ts — THE canonical VestigeOS route registry.
//
// ONE source of truth for every navigation surface. Before this file there were
// four disagreeing inventories (nav-layer's 9-organ COGNITIVE_OS_ROUTES,
// palace-map's 19 ORGAN_REGIONS, MobileNav, and the e2e ALL_ROUTES) — so
// desktop canvas pages became navigation islands (Palace itself was in NONE of
// the visible nav surfaces). Every consumer now derives from this list:
//   - the persistent shell dock          (primary organs)
//   - the grouped command palette (⌘K)   (ALL organs, by group)
//   - the mobile menu                     (ALL groups + organs)
//   - the in-canvas WebGPU nav rail       (enhancement only)
//   - the Palace constellation            (visible organs)
//   - the e2e route smoke list            (all reachable)
//
// A route is only shippable customer-facing nav if it appears here with
// visible !== 'internal'. `_msdftest` is deliberately excluded (internal only).
// ─────────────────────────────────────────────────────────────────────────────

import type { IconName } from '$lib/components/Icon.svelte';

/** Which OS group an organ belongs to (drives palette + mobile grouping). */
export type OsGroup = 'Primary' | 'Understand' | 'Maintain' | 'Reflect' | 'System';

/** How the route mounts its renderer (informational; used by fallback logic). */
export type StageType = 'observatory' | 'route-stage' | 'canvas' | 'dom';

/** Launch visibility: dock = in the persistent dock; nav = in palette/mobile/palace;
 *  hidden = reachable by URL + palette only; internal = never in customer nav. */
export type Visibility = 'dock' | 'nav' | 'hidden' | 'internal';

export interface OsRoute {
	/** Path WITHOUT the base prefix, e.g. '/graph'. Use osHref() to render. */
	href: string;
	/** Human label shown in every nav surface. */
	label: string;
	/** One-line purpose, shown in the command palette + organ overlay. */
	purpose: string;
	group: OsGroup;
	/** Single-key shortcut (⌘K palette + in-canvas rail). Unique across the set. */
	shortcut?: string;
	icon: IconName;
	stage: StageType;
	visibility: Visibility;
	/** true once the organ is launch-ready; false surfaces a "beta" affordance. */
	ready: boolean;
}

// ── The registry — the Palace plus five organs (order = canonical display order) ──
//
// Condensed on 2026-10-15 to fewer, fuller views: 22 routes became the Palace
// plus the full-bleed organs. Every older route redirects to the organ that absorbed it
// (ABSORBED_ROUTES below), so Palace links and bookmarks keep working.
export const OS_ROUTES: OsRoute[] = [
	{ href: '/palace', label: 'Palace', purpose: 'The spatial launcher — every organ as a constellation you fly into.', group: 'Primary', shortcut: 'P', icon: 'logo', stage: 'canvas', visibility: 'dock', ready: true },
	{ href: '/trace', label: 'Trace', purpose: 'Trace your own memory: ask by exact handle, see the receipt, walk the recorded edges back to why.', group: 'Primary', shortcut: 'T', icon: 'trace', stage: 'canvas', visibility: 'dock', ready: true },
	{ href: '/guard', label: 'Guard', purpose: 'What the gate stopped and why: every Operator receipt on its shield, each chained to the one before.', group: 'Primary', shortcut: 'G', icon: 'guard', stage: 'canvas', visibility: 'dock', ready: true },
	{ href: '/field', label: 'Field', purpose: 'The living mind — every memory a light, every recorded edge a filament.', group: 'Primary', shortcut: 'F', icon: 'sparkle', stage: 'observatory', visibility: 'dock', ready: true },
	{ href: '/ledger', label: 'Ledger', purpose: 'The hash-chained record as strata — writes, receipts and intentions through time.', group: 'Primary', shortcut: 'L', icon: 'timeline', stage: 'route-stage', visibility: 'dock', ready: true },
	{ href: '/healing', label: 'Healing', purpose: 'Where memory disagrees with itself or with the code, and the recorded edges that heal it.', group: 'Primary', shortcut: 'H', icon: 'contradictions', stage: 'route-stage', visibility: 'dock', ready: true },
	{ href: '/causal', label: 'Causal', purpose: 'From a failure back to its causes, along recorded edges only.', group: 'Primary', shortcut: 'C', icon: 'reasoning', stage: 'route-stage', visibility: 'dock', ready: true },
	{ href: '/system', label: 'System', purpose: 'The store as it is right now: health, retention, a dream, the durable projection, every tool.', group: 'Primary', shortcut: 'S', icon: 'pulse', stage: 'canvas', visibility: 'dock', ready: true },
	{ href: '/cinema', label: 'Cinema', purpose: 'Memory Cinema — the WebGPU flythrough of your own memory graph.', group: 'Primary', shortcut: 'M', icon: 'cinema', stage: 'canvas', visibility: 'dock', ready: true },

	// Reachable by URL, the palette and the Palace, kept off the dock.
	{ href: '/graph', label: 'Witness', purpose: 'What shaped each decision: its recorded lineage walked back, every witness with its receipt.', group: 'Reflect', icon: 'graph', stage: 'route-stage', visibility: 'hidden', ready: true },
	// One memory, inside: opened from any timeline, a neighbour, or an agent.
	{ href: '/memory', label: 'Memory', purpose: 'One memory, inside: its own words, how strongly it is held, its receipt re-verified live, and every recorded edge to walk on.', group: 'Reflect', icon: 'sparkle', stage: 'canvas', visibility: 'hidden', ready: true },
];

/**
 * Routes that no longer render on their own: each redirects (query string
 * kept) to the organ that absorbed it. Old URLs, Palace links and bookmarks
 * land in the right place without touching any Palace file.
 */
export const ABSORBED_ROUTES: Readonly<Record<string, string>> = Object.freeze({
	'/observatory': '/field',
	'/memories': '/field',
	'/explore': '/field',
	'/activation': '/field',
	'/importance': '/field',
	'/schedule': '/field',
	'/stats': '/field',
	'/timeline': '/ledger',
	'/blackbox': '/ledger',
	'/feed': '/ledger',
	'/dreams': '/ledger',
	'/memory-prs': '/ledger',
	'/intentions': '/ledger',
	'/contradictions': '/healing',
	'/duplicates': '/healing',
	'/patterns': '/healing',
	'/reasoning': '/causal',
	'/settings': '/system'
});

/** The organ an absorbed route now lives in, or undefined for a live route. */
export function absorbedTarget(pathNoBase: string): string | undefined {
	return ABSORBED_ROUTES[pathNoBase];
}

// ── Derived views (every consumer uses these, never re-lists routes) ──────────

/** The persistent desktop dock: primary organs + a Command entry (added by shell). */
export const DOCK_ROUTES: OsRoute[] = OS_ROUTES.filter((r) => r.visibility === 'dock');

/** Everything a customer can navigate to (dock + nav + hidden), for the palette. */
export const NAV_ROUTES: OsRoute[] = OS_ROUTES.filter((r) => r.visibility !== 'internal');

/** Group order for the command palette + mobile menu. */
export const OS_GROUP_ORDER: OsGroup[] = ['Primary', 'Understand', 'Maintain', 'Reflect', 'System'];

/** Routes bucketed by group, in canonical order — drives the palette + mobile menu. */
export function routesByGroup(): { group: OsGroup; routes: OsRoute[] }[] {
	return OS_GROUP_ORDER.map((group) => ({
		group,
		routes: NAV_ROUTES.filter((r) => r.group === group)
	})).filter((g) => g.routes.length > 0);
}

/** The home route — Palace is the actual VestigeOS home. */
export const HOME_ROUTE = '/palace';

/** Look up a route by its href (with or without a leading base). */
export function findRoute(pathNoBase: string): OsRoute | undefined {
	const p = pathNoBase || '/';
	return OS_ROUTES.find((r) => p === r.href || p.startsWith(r.href + '/')) ??
		(p === '/' ? OS_ROUTES.find((r) => r.href === HOME_ROUTE) : undefined);
}
