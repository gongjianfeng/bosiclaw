import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { PageType } from '../../App';
import { RefreshCw, ExternalLink, Loader2, Headset, X, Mail } from 'lucide-react';
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';

interface HeaderProps {
  currentPage: PageType;
}

const pageTitles: Record<PageType, { title: string; description: string }> = {
  dashboard: { title: '概览', description: '服务状态、日志与快捷操作' },
  ai: { title: 'AI 模型配置', description: '配置 AI 提供商和模型' },
  channels: { title: '消息渠道', description: '配置 Telegram、Discord、飞书等' },
  testing: { title: '测试诊断', description: '系统诊断与问题排查' },
  logs: { title: '应用日志', description: '查看 Manager 应用的控制台日志' },
  settings: { title: '设置', description: '身份配置与高级选项' },
};

export function Header({ currentPage }: HeaderProps) {
  const { title, description } = pageTitles[currentPage];
  const [opening, setOpening] = useState(false);
  const [showSupport, setShowSupport] = useState(false);

  const handleOpenDashboard = async () => {
    setOpening(true);
    try {
      const url = await invoke<string>('get_dashboard_url');
      await open(url);
    } catch (e) {
      console.error('打开 Dashboard 失败:', e);
      window.open('http://localhost:18789', '_blank');
    } finally {
      setOpening(false);
    }
  };

  return (
    <>
      <header className="h-14 bg-dark-800/50 border-b border-dark-600 flex items-center justify-between px-6 titlebar-drag backdrop-blur-sm">
        {/* 左侧：页面标题 */}
        <div className="titlebar-no-drag">
          <h2 className="text-lg font-semibold text-white">{title}</h2>
          <p className="text-xs text-gray-500">{description}</p>
        </div>

        {/* 右侧：操作按钮 */}
        <div className="flex items-center gap-2 titlebar-no-drag">
          <button
            onClick={() => setShowSupport(true)}
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-dark-600 hover:bg-dark-500 text-sm text-gray-300 hover:text-white transition-colors"
            title="联系售后"
          >
            <Headset size={14} />
            <span>联系售后</span>
          </button>
          <button
            onClick={() => window.location.reload()}
            className="icon-button text-gray-400 hover:text-white"
            title="刷新"
          >
            <RefreshCw size={16} />
          </button>
          <button
            onClick={handleOpenDashboard}
            disabled={opening}
            className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-dark-600 hover:bg-dark-500 text-sm text-gray-300 hover:text-white transition-colors disabled:opacity-50"
            title="打开 Web Dashboard"
          >
            {opening ? <Loader2 size={14} className="animate-spin" /> : <ExternalLink size={14} />}
            <span>Dashboard</span>
          </button>
        </div>
      </header>

      {/* 联系售后弹窗 */}
      <AnimatePresence>
        {showSupport && (
          <div className="fixed inset-0 z-50 flex items-center justify-center">
            {/* 遮罩层 */}
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              className="absolute inset-0 bg-black/60 backdrop-blur-sm"
              onClick={() => setShowSupport(false)}
            />
            {/* 弹窗内容 */}
            <motion.div
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              transition={{ duration: 0.2 }}
              className="relative bg-dark-800 border border-dark-600 rounded-2xl shadow-2xl p-6 w-72 max-h-[85vh] overflow-y-auto"
            >
              {/* 关闭按钮 */}
              <button
                onClick={() => setShowSupport(false)}
                className="absolute top-4 right-4 p-1 rounded-lg text-gray-500 hover:text-white hover:bg-dark-600 transition-colors"
              >
                <X size={16} />
              </button>

              <div className="text-center">
                <h3 className="text-lg font-semibold text-white mb-1">联系售后服务</h3>
                <p className="text-xs text-gray-500 mb-4">扫描二维码，添加企业微信</p>

                {/* 二维码 */}
                <div className="bg-white rounded-xl p-2 mx-auto mb-4">
                  <img
                    src="/support-qrcode.png"
                    alt="企业微信二维码"
                    className="w-full h-auto rounded-lg"
                  />
                </div>

                {/* 邮箱 */}
                <div className="flex items-center justify-center gap-2 text-sm text-gray-400">
                  <Mail size={14} />
                  <span>bosiclawservice@bosicloud.com</span>
                </div>
              </div>
            </motion.div>
          </div>
        )}
      </AnimatePresence>
    </>
  );
}
