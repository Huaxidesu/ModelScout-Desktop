/**
 * ============================================================================
 * ModelScout —— 前端应用入口
 * ============================================================================
 *
 * 【职责】
 *   1. 创建 Vue 应用实例
 *   2. 安装 Pinia（状态管理）
 *   3. 挂载到 #app
 *
 * 【注意】
 * Naive UI **不需要** app.use() —— 它的组件是自动按需导入的
 * （通过 unplugin-vue-components 或手动 import）。
 * 本项目为了让依赖最小化，采用**手动按需 import 组件**的方式，
 * 详见各组件文件的 import 语句。
 * ============================================================================
 */

import { createApp } from 'vue'
import { createPinia } from 'pinia'

import App from './App.vue'
import './style.css'

const app = createApp(App)

// Pinia 必须在任何 store 被使用前安装
app.use(createPinia())

app.mount('#app')
