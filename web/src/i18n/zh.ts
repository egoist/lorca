import type { Messages } from './en'

export const zh: Messages = {
  meta: {
    title: 'Lorca',
    description:
      '一支住在你自己机器上的 AI 智能体团队。可以单聊，也可以拉群让智能体轮流发言，设备之间通过端到端加密的中继同步。',
  },
  nav: {
    turns: '轮流发言',
    relay: '中继',
    tools: '工具',
    faq: '常见问题',
    docs: '文档',
    download: '下载 Mac 版',
  },
  hero: {
    badge: 'macOS 应用 · CLI 支持 macOS、Linux 和 Windows',
    title: '一支 AI 团队，<br/>住进<accent>你的</accent>电脑。',
    body: '几个 AI 智能体，各自跑在你自己的机器上。和其中一个单聊，把几个拉进群里轮流发言，让它们互相交接工作。每条消息离开你的电脑之前就已经加密。',
    how: '看看它怎么运作',
    platforms: 'macOS 14+ 应用 · CLI 支持 macOS、Linux 和 Windows',
  },
  turns: {
    eyebrow: '群聊',
    title: '人人都有一轮。<accent>不是人人都会开口。</accent>',
    body: '在群里发一条消息，每个智能体会按顺序依次被点到一次。消息是发给它的，或它手里有别人需要的信息，它才回答；否则它就跳过，你一个字也看不见。还有谁要补充，就再来一轮；没人要说了，群里才安静下来。@ 某个名字，那个智能体排到最前；@everyone，所有智能体都会开口。',
    alt: 'Lorca 群聊：Scout 报告了两处信息泄露，Nova 把 schema 变更交给 Patch，Patch 贴出了迁移。',
  },
  relay: {
    eyebrow: '你的机器',
    title: '中继是个信箱，<accent>不是读信的人。</accent>',
    body: '你的身份是在第一台机器上生成的一对密钥，备份就是一串你抄下来的备份短语。用配对字符串把下一台电脑加进来，之后聊天就通过中继同步，而中继里只有密文。轮到另一台机器上的智能体发言时，任务会装进用那台机器的密钥封好的信封里送过去。服务商凭据走的也是同一条路：接入一次服务商，你的其他机器就会收到用账户密钥加密的副本；这把账户密钥，中继从不持有。',
    alt: '三台电脑通过只保存密文的中继互相传递密封的信封。',
    name: '中继',
    machines: ['工作台', '工作室', '壁橱里的 mini'],
    ciphertext: '密文',
  },
  tools: {
    eyebrow: '真正的工具',
    title: '动手干活，<accent>就在你选的机器上。</accent>',
    body: '每个智能体在它的 Runner 上都有一个工作目录，以及AI 编程助手标配的那些工具：read、write、edit、grep、find、ls，还有一个 shell。它用你的权限、在你指定的机器上运行，并告诉你它跑了什么。每个智能体在各次聊天之间还留着自己的记忆，所以你第二次问的时候，它已经知道了。',
    kinds: {
      files: { title: '文件', body: '在你给它的工作目录里打开、新建和修改文件。' },
      search: { title: '搜索', body: '动手之前，先找到那一行、那个文件、那个目录。' },
      shell: { title: 'Shell', body: '以你的权限运行命令：构建、测试、git，活儿需要什么就跑什么。' },
      web: { title: '网络', body: '答案不在磁盘上的时候，就去搜索和阅读网页。' },
      memory: { title: '记忆', body: '跨聊天保留自己的笔记，就是你能直接打开的 Markdown。' },
      plugins: { title: '插件', body: 'GitHub、Linear、Notion，或任何 MCP 服务器。动手之前会先问你。' },
    },
  },
  chef: {
    eyebrow: '第一天',
    title: '从一个智能体开始。<accent>剩下的它来招。</accent>',
    body: '新身份自带 Chef，你的幕僚长。Chef 会问你平时做什么，提议一支各司其职的小团队，你同意后就把它们创建出来。改名、换掉、删掉，都可以。它没什么特别的，只是来得最早。',
    steps: [
      { title: '创建身份', body: '一对密钥，外加十三组备份短语。' },
      { title: '连接服务商', body: '填入 DeepSeek 密钥，或登录 ChatGPT 或 Grok。凭据会加密同步到每台已配对设备。' },
      { title: '认识 Chef', body: '说说你这一周要做的事，然后同意它提议的团队。' },
      { title: '配对下一台机器', body: '粘贴配对字符串，再给它分配一个智能体。' },
    ],
  },
  faq: {
    title: '常见问题',
    items: [
      { q: '我需要服务器吗？', a: '不需要。一台机器就能独立工作。只有在配对第二台设备时才用得上中继，而它只保存密文，别无其他。' },
      { q: '支持哪些平台？', a: '桌面应用支持 Apple 芯片和 Intel Mac。CLI 支持 macOS、Linux 和 Windows，这些平台上的智能体都能加入同一个团队。' },
      { q: '智能体可以用哪些模型？', a: '用 API 密钥接入 DeepSeek，或用你自己的账号登录 ChatGPT 或 Grok。每个智能体分别配置服务商和模型，随时可以改。' },
      { q: '智能体能在我的电脑上做什么？', a: '在你给它的工作目录里读取、写入、编辑文件，搜索，以及运行命令。它用你的权限、在你指定的那台机器上运行。' },
      { q: '中继能看到什么？', a: '加密的数据块、机器的公钥和一个序号。没有名字，没有标题，没有消息内容。' },
    ],
  },
  cta: {
    title: '给你的机器配一支团队。',
    body: '使用 macOS 应用，或在 macOS、Linux 和 Windows 上运行 CLI。',
  },
  docs: {
    title: 'Lorca 文档',
  },
  footer: {
    privacy: '隐私',
    faq: '常见问题',
    download: '下载',
  },
}
