/**
 * ============================================================================
 * 滚动安全的受控 tooltip 状态
 * ============================================================================
 *
 * 【现象】
 * Naive 的浮层（NTooltip 等）teleport 到 body、按坐标跟随触发元素。
 * 触发元素位于可滚动容器内时，滚轮滚动会引发两类缺陷：
 *
 * 1. 浮层跟着触发元素滚出容器可视区——它不受容器 overflow 裁剪，
 *    于是「跳出区域边缘」，悬浮在面板外。
 * 2. 滚动后新行滑到静止的光标下，浏览器会补发 mouseenter（不伴随
 *    mousemove），浮层「滚动收起 → 立刻重开 → 再收起」来回竞速。
 *    过渡状态机一旦在 enter 进行途中被 leave 打断，元素会永久卡在
 *    leave-active：透明度 0 却保留全尺寸与 pointer-events，成为一块
 *    挡住滚轮和悬停的「隐形护盾」。
 *
 * 【修法】
 * 容器一滚动就收起浮层并进入「悬停锁定」；只有用户真正移动鼠标
 * （mousemove）才解锁。滚动补发的 mouseenter 天然不含 mousemove，
 * 被锁挡住，重开竞态随之消失。配套的全局护栏见 style.css 中
 * `.popover-transition-leave-active` 的 pointer-events 规则。
 *
 * 【用法】
 * - 容器（或其可滚动祖先，scroll 不冒泡需 capture）：`@scroll="closeOnScroll"`
 * - 容器：`@mousemove="unlockHover"`
 * - 触发元素：`@mouseenter="enter(key)"`、`@mouseleave="leave"`
 * - 浮层：`trigger="manual"` + `:show="openKey === key"`
 */

import { inject, provide, ref, type InjectionKey } from 'vue'

/** 单例浮层守卫：同一时刻一个容器内只允许一个浮层 */
export type ScrollSafeTooltip = ReturnType<typeof useScrollSafeTooltip>

/** Key 管家列表行的浮层守卫（KeyButlerView 提供，KeyRow 注入） */
export const KEY_LIST_TOOLTIP: InjectionKey<ScrollSafeTooltip> = Symbol('key-list-tooltip')

export function useScrollSafeTooltip() {
  /** 当前显示浮层的触发器标识；`null` 表示没有 */
  const openKey = ref<string | null>(null)

  /** 滚动后的悬停锁定：`true` 时忽略 mouseenter，等待真实鼠标移动 */
  const locked = ref(false)

  /** 绑定到滚动容器的 @scroll（容器自身或其 capture 祖先） */
  function closeOnScroll(): void {
    openKey.value = null
    locked.value = true
  }

  /** 绑定到容器的 @mousemove：真实的鼠标移动才解锁悬停 */
  function unlockHover(): void {
    locked.value = false
  }

  /** 绑定到触发元素的 @mouseenter */
  function enter(key: string): void {
    if (locked.value) return
    openKey.value = key
  }

  /** 绑定到触发元素的 @mouseleave */
  function leave(): void {
    openKey.value = null
  }

  return { openKey, locked, closeOnScroll, unlockHover, enter, leave }
}

/**
 * 在 Key 管家列表中注入守卫。
 *
 * 未提供时（如单元测试单独渲染 KeyRow）退化为本地守卫：
 * 浮层照常开关，只是没有滚动联动。
 */
export function useProvidedListTooltip(): ScrollSafeTooltip {
  return inject(KEY_LIST_TOOLTIP, null) ?? useScrollSafeTooltip()
}

/** KeyButlerView 侧：创建守卫并向列表行提供 */
export function provideListTooltip(): ScrollSafeTooltip {
  const guard = useScrollSafeTooltip()
  provide(KEY_LIST_TOOLTIP, guard)
  return guard
}
