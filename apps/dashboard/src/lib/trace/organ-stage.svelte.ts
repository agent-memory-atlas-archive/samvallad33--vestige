/**
 * One field for every organ built on Trace.
 *
 * The (app) layout renders a single TraceStage while any organ page owns it,
 * so moving between Trace, Ledger, Guard, Healing, Causal and System keeps the
 * same GPU field: the next organ's story starts from the particles already on
 * screen (TracePass.continueNext) instead of a black canvas and a new boot.
 * Pages describe what the stage shows through OrganStage, which writes here.
 * Every field is its own signal (raw, never proxied), so a page changing its
 * picked memory never re-runs the stage's answer.
 */
import type { TraceLink, TraceMemory, WalkResult } from './trace-data';
import type { Axis, Story, VerifyState } from './trace-pass';
import type { StageWords } from './TraceStage.svelte';

class OrganStageState {
	owner = $state.raw<object | null>(null);
	memories = $state.raw<TraceMemory[]>([]);
	range = $state.raw<{ start: number; end: number } | null>(null);
	links = $state.raw<TraceLink[]>([]);
	walk = $state.raw<WalkResult | null>(null);
	selectedId = $state.raw<string | null>(null);
	verify = $state.raw<VerifyState>('idle');
	now = $state.raw<number | null>(null);
	origin = $state.raw<(() => { x: number; y: number } | undefined) | undefined>(undefined);
	describe = $state.raw<((id: string) => StageWords | null) | undefined>(undefined);
	labelIds = $state.raw<string[]>([]);
	onpick = $state.raw<((id: string, kind: 'answer' | 'walk') => void) | undefined>(undefined);
	seed = $state.raw('vestige-trace-v1');
	quiet = $state.raw(false);
	axis = $state.raw<Axis>('time');
	story = $state.raw<Story>('trace');
	enter = $state.raw<boolean | 'second'>(true);
}

export const organStage = new OrganStageState();

export function claimStage(owner: object) {
	organStage.owner = owner;
}

/**
 * Let go of the stage, unless another organ already took it over: the next
 * page mounts in the same navigation, so the release waits a task to see.
 */
export function releaseStage(owner: object) {
	setTimeout(() => {
		if (organStage.owner === owner) organStage.owner = null;
	}, 0);
}
