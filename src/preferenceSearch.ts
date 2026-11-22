import type {PreferencesPage} from './PreferencesScreen';
export type PreferenceSearchResult={page:PreferencesPage;label:string;keywords:string};
// Only list controls connected to an implemented preference.
export const preferenceSearchItems:PreferenceSearchResult[]=[
 ...['Main/Master','Develop','Feature','Release','Hotfix','Version Tag'].map(label=>({page:'gitflow' as const,label:`Gitflow ${label}`,keywords:'gitflow branches prefixes 깃플로우 브랜치 접두사'})),
 {page:'integrations',label:'GitHub repository',keywords:'provider integrations github authentication token 통합 인증 저장소'},
 {page:'external',label:'External Editor',keywords:'vscode code editor external 외부 에디터 편집기'},
 {page:'external',label:'External Terminal',keywords:'terminal shell cmd 외부 터미널'},
 {page:'general',label:'Lazy Load Commits in Graph',keywords:'scroll graph history lazy 그래프 스크롤 자동 로딩'},
 {page:'general',label:'Initial Commits in Graph',keywords:'graph history page size commits 그래프 커밋 개수'},
 {page:'general',label:'Auto-Prune',keywords:'fetch remote stale tracking prune 원격 추적 정리'},
 {page:'general',label:'Default Branch Name',keywords:'branch main init 기본 브랜치'},
 {page:'general',label:'Remember tabs',keywords:'startup restore repository 탭 복원'},
 {page:'general',label:'Launchpad Start View',keywords:'launchpad repositories pulls issues 시작 화면'},
 {page:'profiles',label:'Default author name',keywords:'profile identity commit 이름 프로필'},
 {page:'profiles',label:'Default author email',keywords:'profile identity commit 이메일'},
 {page:'ui',label:'Theme',keywords:'dark light system 테마'},
 {page:'ui',label:'Notification Location',keywords:'toast corner 알림 위치'},
 {page:'ui',label:'Date/Time Locale',keywords:'language date time 날짜 언어'},
 {page:'ui',label:'Date Format',keywords:'date format 날짜 형식'},
 {page:'ui',label:'Show toolbar icon labels',keywords:'toolbar label 툴바 도구 이름'},
 {page:'ui',label:'Reference sidebar',keywords:'panel layout sidebar 사이드바'},
 {page:'editor',label:'Editor font',keywords:'font monospace 글꼴 폰트'},
 {page:'editor',label:'Editor font size',keywords:'font size 글자 크기'},
 {page:'editor',label:'Editor tab size',keywords:'tab indent 탭 들여쓰기'},
 {page:'editor',label:'Show Line Numbers',keywords:'line number 줄 번호'},
 {page:'editor',label:'Word Wrap',keywords:'wrap line 줄 바꿈'},
 {page:'editor',label:'Preferred diff layout',keywords:'diff split unified 비교 분할'},
 {page:'commit',label:'Commit template summary',keywords:'message template summary 커밋 템플릿 제목'},
 {page:'commit',label:'Commit template description',keywords:'message template body 커밋 템플릿 본문'},
 {page:'commit',label:'Remove comments from commit messages',keywords:'comments hash 커밋 주석'},
];
export function findPreferences(query:string,repositoryAvailable:boolean){
 const terms=query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
 return terms.length?preferenceSearchItems.filter(item=>(repositoryAvailable||!['commit','gitflow'].includes(item.page))&&terms.every(term=>`${item.label} ${item.keywords}`.toLocaleLowerCase().includes(term))):[];
}
