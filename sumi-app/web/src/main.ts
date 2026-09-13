import { createApp } from 'vue';
import { createPinia } from 'pinia';
import App from './app/App.vue';
import './app/styles.css';

createApp(App).use(createPinia()).mount('#app');
