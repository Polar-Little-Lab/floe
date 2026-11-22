import { afterEach, expect, test, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import App from './App';
import { demo } from './demo';
import type {Snapshot} from './types';

afterEach(() => vi.unstubAllGlobals());

test('all retained AI and Agents controls are disabled and explain the exclusion', () => {
  vi.stubGlobal('localStorage', { getItem: () => null });
  const html = renderToStaticMarkup(<App/>);
  const buttons = html.match(/<button\b[\s\S]*?<\/button>/g) ?? [];
  const aiControls = buttons.filter(button => /class="ai-button\b|>Agents<\/button>/.test(button));
  expect(aiControls).toHaveLength(4);
  for (const button of aiControls) {
    expect(button).toContain('disabled=""');
    expect(button).toContain('AI 기능은 구현 대상에서 제외되었습니다.');
  }
});

test('reference fixture tags resolve to the graph commit carrying the same label', () => {
  for (const tag of demo.tags) {
    expect(demo.commits.find(commit => commit.oid === tag.oid)?.refs).toContain(tag.name);
  }
});

test.each([
  {replayDraft:{version:1,branch:'main',head:'a'.repeat(40),source:'b'.repeat(40),kind:'cherryPick',mainline:null,resolutions:{}}},
  {replayDraftError:'Damaged replay draft'},
  {mergeDraft:{name:'topic',branch:'main',head:'a'.repeat(40),target:'b'.repeat(40),resolutions:{}}},
  {mergeDraftError:'Damaged merge draft'},
] satisfies Partial<Snapshot>[])('pending resolution blocks ordinary writes while recovery remains available: %j', state => {
  vi.stubGlobal('localStorage', {getItem:()=>null});
  const original={...demo};
  try {
    Object.assign(demo,{repoId:'isolated-fixture',writeBlock:null},state);
    const html=renderToStaticMarkup(<App/>);
    const buttons=html.match(/<button\b[\s\S]*?<\/button>/g)??[];
    for(const label of ['Stage All Changes','Unstage All Changes']) {
      expect(buttons.find(b=>b.includes(`>${label}</button>`))).toContain('disabled=""');
    }
    expect(html.match(/<label class="amend"><input[^>]*>/)?.[0]).toContain('disabled=""');
    const recovery=buttons.filter(b=>/>(Resume replay|Abort replay|Resume merge|Abort merge|Archive damaged (replay|merge) draft)<\/button>/.test(b));
    expect(recovery.length).toBeGreaterThan(0);
    for(const button of recovery)expect(button).not.toContain('disabled=""');
  } finally {
    for(const key of Object.keys(state))delete demo[key as keyof Snapshot];
    Object.assign(demo,original);
  }
});


test.each([
  {sequenceProgress:{id:'sequence',branch:'main',originalHead:'a'.repeat(40),expectedHead:'a'.repeat(40),total:2,completed:0,pending:true}},
  {sequenceError:'INVALID_SEQUENCE_STATE: <damaged>'},
] satisfies Partial<Snapshot>[])('sequence state disables writes and displays progress or damage: %j', state => {
  vi.stubGlobal('localStorage', {getItem:()=>null});
  const original={...demo};
  try {
    Object.assign(demo,{repoId:'isolated-fixture',writeBlock:null},state);
    const html=renderToStaticMarkup(<App/>);
    const buttons=html.match(/<button\b[\s\S]*?<\/button>/g)??[];
    for(const label of ['Stage All Changes','Unstage All Changes']) {
      expect(buttons.find(b=>b.includes(`>${label}</button>`))).toContain('disabled=""');
    }
    expect(html.match(/<label class="amend"><input[^>]*>/)?.[0]).toContain('disabled=""');
    expect(html).toContain('other writes are blocked');
    if(state.sequenceProgress) {
      expect(html).toContain('0/2 steps completed');
      expect(html).toContain('Step awaiting completion');
    } else {
      expect(html).toContain('&lt;damaged&gt;');
      expect(html).not.toContain('<damaged>');
    }
  } finally {
    for(const key of Object.keys(state))delete demo[key as keyof Snapshot];
    Object.assign(demo,original);
  }
});
