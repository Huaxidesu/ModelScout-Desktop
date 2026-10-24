/**
 * 前端应用入口：创建 Vue 应用、安装 Pinia、挂载到 #app。
 *
 * Naive UI 组件采用手动按需 import（不做 app.use 全局注册），以减小依赖体积；
 * 见各组件文件的 import 语句。
 */

import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import './style.css'

const app = createApp(App)

// Pinia 必须在任何 store 被使用前安装
app.use(createPinia())

app.mount('#app')
