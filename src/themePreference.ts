export type ThemePreference = 'dark' | 'light' | 'system';
export type ResolvedTheme = 'dark' | 'light';
export const THEME_KEY = 'branchdesk-theme';
export const isThemePreference = (value: unknown): value is ThemePreference => value === 'dark' || value === 'light' || value === 'system';
type ThemeStorage = Pick<Storage, 'getItem' | 'setItem'>;
export function loadTheme(storage: () => ThemeStorage): {theme: ThemePreference; warning: string} {
 try {
  const value=storage().getItem(THEME_KEY);
  if(value===null)return {theme:'dark',warning:''};
  if(isThemePreference(value))return {theme:value,warning:''};
  return {theme:'dark',warning:'Saved theme was invalid. Choose a theme to replace it.'};
 } catch {
  return {theme:'dark',warning:'Saved theme could not be read. Theme changes may only last for this session.'};
 }
}
export function saveTheme(storage: () => ThemeStorage, theme: ThemePreference): string {
 try {storage().setItem(THEME_KEY,theme);return '';}
 catch {return 'Theme changed for this session, but could not be saved. Try selecting it again.';}
}
type ThemeQuery = Pick<MediaQueryList,'matches'|'addEventListener'|'removeEventListener'>;
// Keep system observation scoped to the setting's lifetime, including StrictMode cleanup.
export function observeTheme(preference: ThemePreference, match: () => ThemeQuery, apply: (theme: ResolvedTheme) => void): () => void {
 if(preference!=='system'){apply(preference);return ()=>{};}
 try {
  const query=match();
  const update=()=>apply(query.matches?'dark':'light');
  query.addEventListener('change',update);
  update();
  return ()=>query.removeEventListener('change',update);
 } catch {apply('dark');return ()=>{};}
}
