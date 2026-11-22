import type { Commit } from './types';
export interface Edge { from: number; to: number; color: number }
export interface GraphRow { lane: number; color: number; incoming: Edge[]; edges: Edge[]; width: number }
/** Keep each branch lane alive until its target commit is actually visited. */
export function buildGraph(commits: Commit[]): GraphRow[] {
 let lanes: (string|null)[]=[];
 return commits.map(commit=>{
  const before=[...lanes];
  let lane=lanes.indexOf(commit.oid);
  if(lane<0){lane=lanes.indexOf(null);if(lane<0)lane=lanes.length;}
  const incoming:Edge[]=[];
  before.forEach((target,from)=>{if(target!==null)incoming.push({from,to:target===commit.oid?lane:from,color:from%6});});
  lanes=lanes.map(target=>target===commit.oid?null:target);
  if(lanes.length<=lane)lanes.length=lane+1;
  lanes[lane]=null;
  const parents:number[]=[];
  commit.parents.forEach((parent,i)=>{
   let slot=i===0?lane:lanes.indexOf(null);
   if(slot<0)slot=lanes.length;
   lanes[slot]=parent;parents.push(slot);
  });
  const edges:Edge[]=[];
  before.forEach((target,from)=>{if(target!==null&&target!==commit.oid)edges.push({from,to:from,color:from%6});});
  parents.forEach(slot=>edges.push({from:lane,to:slot,color:slot%6}));
  while(lanes.length&&lanes[lanes.length-1]===null)lanes.pop();
  return{lane,color:lane%6,incoming,edges,width:Math.max(before.length,lanes.length,lane+1)};
 });
}
