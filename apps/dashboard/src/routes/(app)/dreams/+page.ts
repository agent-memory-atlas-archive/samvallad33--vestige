import type { PageLoad } from './$types';
import { redirectAbsorbed } from '$lib/absorbed-redirect';

// Absorbed by the condensed dashboard (see ABSORBED_ROUTES in os-routes.ts).
export const load: PageLoad = ({ url }) => redirectAbsorbed('/dreams', url);
