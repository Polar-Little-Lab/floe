import {AppErrorBoundary} from './AppErrorBoundary';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './original-theme.css';
import './styles.css';
import './light-theme.css';
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><AppErrorBoundary><App/></AppErrorBoundary></React.StrictMode>);
