/**
 * ============================================================================
 * ModelScout —— provider 命名推断单元测试
 * ============================================================================
 *
 * 命名推断是「生成配置」这条链上最容易出错、又最容易被忽略的一环：
 * 它错了不会报错，只会让用户拿到一份 key 不直观的配置。
 * 尤其是 `api.openai.com` 被推成 `api` 这类「差一点」的结果，
 * 只有靠用例才能锁住。
 * ============================================================================
 */

import { describe, expect, it } from 'vitest'

import {
  hasApiKey,
  inferEnvVarName,
  inferProviderKey,
  inferProviderName,
  resolveApiKeyField,
  sanitizeEnvVarName,
  sanitizeProviderKey,
} from './naming'

describe('inferProviderKey —— 从 Base URL 推断 provider 键名', () => {
  it('跳过 api/www 之类的通用子域，取域名主体', () => {
    expect(inferProviderKey('https://api.openai.com/v1')).toBe('openai')
    expect(inferProviderKey('https://www.deepseek.com/v1')).toBe('deepseek')
  })

  it('吃掉两段公共后缀（org.cn / co.uk 等）', () => {
    // discovery-api.intern-ai.org.cn → 主体是 intern-ai，不是 org
    expect(inferProviderKey('https://discovery-api.intern-ai.org.cn/v1')).toBe('intern-ai')
    expect(inferProviderKey('https://api.example.co.uk/v1')).toBe('example')
  })

  it('局域网与回环地址一律推成 local', () => {
    expect(inferProviderKey('http://192.168.66.202:8765/v1')).toBe('local')
    expect(inferProviderKey('http://127.0.0.1:11434/v1')).toBe('local')
    expect(inferProviderKey('http://localhost:8000/v1')).toBe('local')
    expect(inferProviderKey('http://[::1]:8080/v1')).toBe('local')
  })

  it('容忍没有协议头的裸地址', () => {
    // 用户经常直接粘 "192.168.66.202:8765/v1" 这种片段
    expect(inferProviderKey('192.168.66.202:8765/v1')).toBe('local')
    expect(inferProviderKey('api.moonshot.cn/v1')).toBe('moonshot')
  })

  it('把非法字符规整成连字符', () => {
    expect(inferProviderKey('https://my_llm.gateway.example.com/v1')).toBe('example')
    expect(inferProviderKey('https://api.内部.测试/v1')).toBe('modelscout')
  })

  it('解析不出任何东西时回退到 modelscout，而不是抛异常', () => {
    expect(inferProviderKey('')).toBe('modelscout')
    expect(inferProviderKey('   ')).toBe('modelscout')
    expect(inferProviderKey('https://')).toBe('modelscout')
    expect(inferProviderKey('com')).toBe('modelscout')
  })
})

describe('inferProviderName —— 键名 → 显示名', () => {
  it('已知品牌保持厂商自己的写法', () => {
    // 纯 title case 会得到 "Deepseek" / "Openai"，明显是机器生成的
    expect(inferProviderName('deepseek-local')).toBe('DeepSeek Local')
    expect(inferProviderName('openai')).toBe('OpenAI')
    expect(inferProviderName('glm')).toBe('GLM')
    expect(inferProviderName('modelscout')).toBe('ModelScout')
  })

  it('未知词走首字母大写', () => {
    expect(inferProviderName('my-gateway')).toBe('My Gateway')
    expect(inferProviderName('intern-ai')).toBe('Intern AI')
  })

  it('下划线、点、空格都当分隔符', () => {
    expect(inferProviderName('my_gateway_v2')).toBe('My Gateway V2')
    expect(inferProviderName('my.gateway')).toBe('My Gateway')
    expect(inferProviderName('my gateway')).toBe('My Gateway')
  })

  it('空输入回退到 ModelScout', () => {
    expect(inferProviderName('')).toBe('ModelScout')
    expect(inferProviderName('---')).toBe('ModelScout')
  })
})

describe('inferEnvVarName —— 键名 → 环境变量名', () => {
  it('拼出大写下划线形式', () => {
    expect(inferEnvVarName('deepseek-local')).toBe('DEEPSEEK_LOCAL_API_KEY')
    expect(inferEnvVarName('my.gateway')).toBe('MY_GATEWAY_API_KEY')
  })

  it('与官方惯例一致，可直接复用已有变量', () => {
    expect(inferEnvVarName('openai')).toBe('OPENAI_API_KEY')
    expect(inferEnvVarName('anthropic')).toBe('ANTHROPIC_API_KEY')
  })

  it('空输入回退到默认变量名', () => {
    expect(inferEnvVarName('')).toBe('MODELSCOUT_API_KEY')
  })
})

describe('sanitizeProviderKey —— 键名规整', () => {
  it('小写化并替换非法字符', () => {
    expect(sanitizeProviderKey('DeepSeek Local')).toBe('deepseek-local')
    expect(sanitizeProviderKey('My@LLM!Gateway')).toBe('my-llm-gateway')
  })

  it('合并连续连字符、去掉首尾分隔符', () => {
    expect(sanitizeProviderKey('a---b')).toBe('a-b')
    expect(sanitizeProviderKey('--x--')).toBe('x')
  })

  it('保留点与下划线（它们是合法的键名字符）', () => {
    expect(sanitizeProviderKey('a.b_c')).toBe('a.b_c')
  })
})

describe('sanitizeEnvVarName —— 环境变量名规整', () => {
  it('只保留 A-Z0-9_', () => {
    expect(sanitizeEnvVarName('my-key api')).toBe('MY_KEY_API')
    expect(sanitizeEnvVarName('a.b.c')).toBe('A_B_C')
  })

  it('全非法字符时回退到默认值', () => {
    expect(sanitizeEnvVarName('---')).toBe('MODELSCOUT_API_KEY')
  })
})

describe('resolveApiKeyField —— 密钥写入形态', () => {
  it('plain 模式返回明文（去首尾空白）', () => {
    expect(resolveApiKeyField('plain', '  sk-abc  ', 'X_API_KEY')).toBe('sk-abc')
  })

  it('env 模式返回 opencode 的 {env:VAR} 引用，忽略明文', () => {
    expect(resolveApiKeyField('env', 'sk-abc', 'DEEPSEEK_LOCAL_API_KEY')).toBe(
      '{env:DEEPSEEK_LOCAL_API_KEY}',
    )
  })

  it('env 模式下环境变量名非法时会被规整', () => {
    expect(resolveApiKeyField('env', '', 'my key')).toBe('{env:MY_KEY}')
  })

  it('plain 模式未填密钥时返回空串（调用方据此省略字段）', () => {
    expect(resolveApiKeyField('plain', '', 'X_API_KEY')).toBe('')
    expect(resolveApiKeyField('plain', '   ', 'X_API_KEY')).toBe('')
  })
})

describe('hasApiKey —— 是否已填密钥', () => {
  it('非空即算已填（刻意不校验格式）', () => {
    expect(hasApiKey('sk-abc')).toBe(true)
    expect(hasApiKey('glm-xyz')).toBe(true)
    expect(hasApiKey('deadbeef')).toBe(true)
  })

  it('空白与 undefined 算未填', () => {
    expect(hasApiKey('')).toBe(false)
    expect(hasApiKey('   ')).toBe(false)
    expect(hasApiKey(undefined)).toBe(false)
  })
})
