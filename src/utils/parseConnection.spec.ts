/**
 * ============================================================================
 * ModelScout —— 连接信息识别单元测试
 * ============================================================================
 *
 * 这个模块的失败模式分两种，测试对两者都做了覆盖：
 *   1. **漏识别**（文本明明有端点/密钥却未提取出来）—— 影响体验但无危害
 *   2. **错识别**（把文档链接当端点、把域名当密钥）—— 危险，
 *      用户会拿着错误的值去测试连接，排查成本高
 * 第 2 类用例是重点。
 * ============================================================================
 */

import { describe, expect, it } from 'vitest'

import { extractConnection, looksLikeHostname, maskSecret, scoreUrl } from './parseConnection'

describe('extractConnection —— 真实粘贴场景', () => {
  it('两行裸文本：URL 与 sk- 密钥各占一行', () => {
    const result = extractConnection(
      'https://discovery-api.intern-ai.org.cn/v1\nsk-80902197687b60084c6f72aaddd8b887014689ebce218dde31e9324fe1c58aab',
    )
    expect(result.baseUrl).toBe('https://discovery-api.intern-ai.org.cn/v1')
    expect(result.apiKey).toBe(
      'sk-80902197687b60084c6f72aaddd8b887014689ebce218dde31e9324fe1c58aab',
    )
  })

  it('聊天口吻的整段话（用户真实给过的样子）', () => {
    const result = extractConnection(
      '给你测试链接，不用担心泄密，我后续会删除api\n' +
        'https://discovery-api.intern-ai.org.cn/v1    ' +
        'sk-d37b43614ad2f6f7faaaad2f4abe77fb69e2d95bd3fde7aa6828175524868ddf',
    )
    expect(result.baseUrl).toBe('https://discovery-api.intern-ai.org.cn/v1')
    expect(result.apiKey).toBe(
      'sk-d37b43614ad2f6f7faaaad2f4abe77fb69e2d95bd3fde7aa6828175524868ddf',
    )
  })

  it('带标签的文档式文本', () => {
    const result = extractConnection(
      'Base URL: https://api.moonshot.cn/v1\nAPI Key: sk-abcdefghijklmnopqrstuvwx',
    )
    expect(result.baseUrl).toBe('https://api.moonshot.cn/v1')
    expect(result.apiKey).toBe('sk-abcdefghijklmnopqrstuvwx')
  })

  it('中文标签与全角冒号', () => {
    const result = extractConnection(
      '接口地址：https://open.bigmodel.cn/api/paas/v4\n密钥：abcdef0123456789abcdef0123456789',
    )
    expect(result.baseUrl).toBe('https://open.bigmodel.cn/api/paas/v4')
    expect(result.apiKey).toBe('abcdef0123456789abcdef0123456789')
  })

  it('JSON 片段（字段名对但值没有明显特征）', () => {
    const result = extractConnection(
      JSON.stringify(
        {
          options: {
            baseURL: 'https://api.siliconflow.cn/v1',
            apiKey: 'abcdefghijklmnopqrstuvwx',
          },
        },
        null,
        2,
      ),
    )
    expect(result.baseUrl).toBe('https://api.siliconflow.cn/v1')
    expect(result.apiKey).toBe('abcdefghijklmnopqrstuvwx')
  })

  it('Bearer 令牌（同时把完整接口地址收敛成 Base URL）', () => {
    const result = extractConnection(
      'curl -H "Authorization: Bearer abcdefghijklmnopqrst" https://api.deepseek.com/v1/models',
    )
    expect(result.baseUrl).toBe('https://api.deepseek.com/v1')
    expect(result.apiKey).toBe('abcdefghijklmnopqrst')
  })

  it('完整接口地址会被收敛成 Base URL（否则 AI SDK 会拼出双份路径）', () => {
    expect(extractConnection('https://api.deepseek.com/v1/chat/completions').baseUrl).toBe(
      'https://api.deepseek.com/v1',
    )
    expect(extractConnection('https://api.openai.com/v1/embeddings').baseUrl).toBe(
      'https://api.openai.com/v1',
    )
    // 中文句号也要能先被剥掉，再收敛路径
    expect(extractConnection('端点：https://api.deepseek.com/v1/models。').baseUrl).toBe(
      'https://api.deepseek.com/v1',
    )
  })
})

describe('extractConnection —— 误判防护（重点）', () => {
  it('文本里混着文档链接时，选中的必须是真正的 API 端点', () => {
    const result = extractConnection(
      '参考 https://opencode.ai/docs/config/ 与 https://github.com/sst/opencode\n' +
        'Base URL: https://api.moonshot.cn/v1\n' +
        'Key: sk-abcdefghijklmnopqrstuvwx',
    )
    expect(result.baseUrl).toBe('https://api.moonshot.cn/v1')
    expect(result.urlCandidates[0]).toBe('https://api.moonshot.cn/v1')
  })

  it('指向 .md 文档的链接不会被当成端点', () => {
    const result = extractConnection('看这个 https://example.com/docs/guide.md')
    expect(result.baseUrl).toBe('')
  })

  it('域名不会被当成密钥', () => {
    // 密钥正则的字符集覆盖了域名，必须靠 looksLikeHostname 拦住
    const result = extractConnection(
      'secret-key: discovery-api.intern-ai.org.cn\n' + 'sk-abcdefghijklmnopqrstuvwx',
    )
    expect(result.apiKey).toBe('sk-abcdefghijklmnopqrstuvwx')
  })

  it('落在 URL 内部的片段不会被当成密钥', () => {
    const result = extractConnection('https://discovery-api.intern-ai.org.cn/v1')
    expect(result.apiKey).toBe('')
    expect(result.baseUrl).toBe('https://discovery-api.intern-ai.org.cn/v1')
  })

  it('本项目的 dev server 地址会被排除', () => {
    const result = extractConnection('开发时用 http://localhost:1420 调试')
    expect(result.baseUrl).toBe('')
  })

  it('句末标点不会被粘进 URL', () => {
    const result = extractConnection('端点在这里：https://api.openai.com/v1。')
    expect(result.baseUrl).toBe('https://api.openai.com/v1')
  })

  it('尖括号包裹的 URL 也能正确剥离', () => {
    const result = extractConnection('地址是 <https://api.deepseek.com/v1>')
    expect(result.baseUrl).toBe('https://api.deepseek.com/v1')
  })
})

describe('extractConnection —— 边界与说明文案', () => {
  it('空输入返回空结果并给出提示', () => {
    const result = extractConnection('   ')
    expect(result.baseUrl).toBe('')
    expect(result.apiKey).toBe('')
    expect(result.notes.join()).toContain('没有输入')
  })

  it('只有密钥没有地址时，部分识别并说明缺了什么', () => {
    const result = extractConnection('sk-abcdefghijklmnopqrstuvwx')
    expect(result.apiKey).toBe('sk-abcdefghijklmnopqrstuvwx')
    expect(result.baseUrl).toBe('')
    expect(result.notes.join()).toContain('没找到 Base URL')
  })

  it('只有地址没有密钥时，部分识别并说明缺了什么', () => {
    const result = extractConnection('https://api.openai.com/v1')
    expect(result.baseUrl).toBe('https://api.openai.com/v1')
    expect(result.apiKey).toBe('')
    expect(result.notes.join()).toContain('没找到 API Key')
  })

  it('地址缺少版本段时给出可执行的提示', () => {
    const result = extractConnection('https://api.example.com/openai')
    expect(result.notes.join()).toContain('/v1')
  })

  it('多个端点时选分数最高的，并在说明里提及其它候选', () => {
    const result = extractConnection(
      'https://api.example.com/health\nhttps://api.example.com/v1\n',
    )
    expect(result.baseUrl).toBe('https://api.example.com/v1')
    expect(result.notes.join()).toContain('另有候选')
  })

  it('不把 sk- 密钥的十六进制尾巴重复算成一个候选', () => {
    const result = extractConnection('sk-80902197687b60084c6f72aaddd8b887014689ebce218dde31e9324fe1c58aab')
    expect(result.keyCandidates).toHaveLength(1)
    expect(result.notes.join()).not.toContain('另有')
  })
})

describe('scoreUrl —— 打分规则', () => {
  it('版本路径与 api 主机名加分', () => {
    expect(scoreUrl('https://api.openai.com/v1')).toBeGreaterThan(
      scoreUrl('https://openai.com'),
    )
  })

  it('本地与内网地址加分', () => {
    expect(scoreUrl('http://192.168.66.202:8765/v1')).toBeGreaterThan(0)
    expect(scoreUrl('http://127.0.0.1:11434/v1')).toBeGreaterThan(0)
  })

  it('文档与代码托管站点被扣成负分', () => {
    expect(scoreUrl('https://github.com/sst/opencode')).toBeLessThanOrEqual(0)
    expect(scoreUrl('https://opencode.ai/docs/config/')).toBeLessThanOrEqual(0)
    expect(scoreUrl('https://example.com/readme.md')).toBeLessThanOrEqual(0)
  })
})

describe('looksLikeHostname', () => {
  it('识别出域名', () => {
    expect(looksLikeHostname('discovery-api.intern-ai.org.cn')).toBe(true)
    expect(looksLikeHostname('api.moonshot.cn')).toBe(true)
    expect(looksLikeHostname('my-gateway.example.com')).toBe(true)
  })

  it('不误伤常见密钥形态', () => {
    expect(looksLikeHostname('sk-80902197687b60084c6f72aaddd8b887014689ebce218dde31e9324fe1c58aab')).toBe(false)
    expect(looksLikeHostname('abcdef0123456789abcdef0123456789')).toBe(false)
    // GLM 形态：id.secret
    expect(looksLikeHostname('1a2b3c4d5e6f7890.abcdef1234567890')).toBe(false)
  })
})

describe('maskSecret —— 脱敏展示', () => {
  it('长密钥只露头尾', () => {
    const masked = maskSecret('sk-80902197687b60084c6f72aaddd8b887014689ebce218dde31e9324fe1c58aab')
    expect(masked).toBe('sk-809…8aab')
    expect(masked).not.toContain('80902197')
  })

  it('短串整体打码，不泄露长度之外的任何信息', () => {
    expect(maskSecret('short')).toBe('***')
    expect(maskSecret('')).toBe('')
  })
})
