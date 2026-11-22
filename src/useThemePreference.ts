import {useEffect,useState} from 'react';
import {isThemePreference,loadTheme,observeTheme,saveTheme,THEME_KEY,type ThemePreference} from './themePreference';
export function useThemePreference(){
 const [state,setState]=useState(()=>loadTheme(()=>window.localStorage));
 useEffect(()=>observeTheme(state.theme,()=>window.matchMedia('(prefers-color-scheme: dark)'),theme=>{document.documentElement.dataset.theme=theme;document.documentElement.style.colorScheme=theme;}),[state.theme]);
 useEffect(()=>{
  const changed=(event:StorageEvent)=>{
   if(event.key!==THEME_KEY&&event.key!==null)return;
   // A storage event is observation, never a write-back (avoids cross-window loops).
   try {if(event.storageArea!==window.localStorage)return;}catch{return;}
   setState(loadTheme(()=>window.localStorage));
  };
  window.addEventListener('storage',changed);
  return ()=>window.removeEventListener('storage',changed);
 },[]);
 const choose=(theme:ThemePreference)=>{if(isThemePreference(theme))setState({theme,warning:saveTheme(()=>window.localStorage,theme)});};
 return {theme:state.theme,themeWarning:state.warning,chooseTheme:choose};
}
