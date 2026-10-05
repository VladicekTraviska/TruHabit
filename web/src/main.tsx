import { createRoot } from 'react-dom/client';
import { App } from './App';
import './style.css';
import './product.css';
import './design.css';
import { getLanguage, setLanguage } from './i18n';
setLanguage(getLanguage());
createRoot(document.getElementById('root')!).render(<App />);
