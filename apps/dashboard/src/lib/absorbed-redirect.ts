// Absorbed routes — the 2026-10-15 condensation folded 22 dashboard routes into
// the Palace plus five organs. A route that no longer renders on its own keeps
// a tiny load function that sends the visitor (query string intact, so
// ?memory=, ?q= and ?run= survive) to the organ that absorbed it.
import { redirect } from '@sveltejs/kit';
import { base } from '$app/paths';
import { HOME_ROUTE, absorbedTarget } from '$lib/os-routes';

export function redirectAbsorbed(from: string, url: URL): never {
	const to = absorbedTarget(from) ?? HOME_ROUTE;
	redirect(308, `${base}${to}${url.search}`);
}
