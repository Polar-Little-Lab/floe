import {createContext,useContext} from 'react';
export type LaunchpadFilterScope='pulls'|'issues'|'workspace';
type Slots={scope:LaunchpadFilterScope|null;filters:HTMLElement|null;refresh:HTMLElement|null};
export const LaunchpadSlots=createContext<Slots|null>(null);
export function useLaunchpadSlots(scope?:LaunchpadFilterScope){const slots=useContext(LaunchpadSlots);return scope&&slots?.scope===scope?slots:null;}
