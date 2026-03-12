import { useEffect } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Mail } from 'lucide-react';

interface SupportModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const backdropVariants = {
  hidden: { opacity: 0 },
  visible: { opacity: 1 },
};

const modalVariants = {
  hidden: { opacity: 0, scale: 0.95 },
  visible: {
    opacity: 1,
    scale: 1,
    transition: { duration: 0.2 },
  },
};

export function SupportModal({ isOpen, onClose }: SupportModalProps) {
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  return createPortal(
    <AnimatePresence>
      {isOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center" role="dialog" aria-modal="true">
          <motion.div
            variants={backdropVariants}
            initial="hidden"
            animate="visible"
            exit="hidden"
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={onClose}
          />
          <motion.div
            variants={modalVariants}
            initial="hidden"
            animate="visible"
            exit="hidden"
            className="relative bg-dark-800 border border-dark-600 rounded-2xl shadow-2xl p-6 w-72 max-h-[85vh] overflow-y-auto"
          >
            <button
              onClick={onClose}
              className="absolute top-4 right-4 p-1 rounded-lg text-gray-500 hover:text-white hover:bg-dark-600 transition-colors"
            >
              <X size={16} />
            </button>
            <div className="text-center">
              <h3 className="text-lg font-semibold text-white mb-1">联系售后服务</h3>
              <p className="text-xs text-gray-500 mb-4">扫描二维码，添加企业微信</p>
              <div className="bg-white rounded-xl p-2 mx-auto mb-4">
                <img
                  src="/support-qrcode.png"
                  alt="企业微信二维码"
                  className="w-full h-auto rounded-lg"
                />
              </div>
              <div className="flex items-center justify-center gap-2 text-sm text-gray-400">
                <Mail size={14} />
                <span>bosiclawservice@bosicloud.com</span>
              </div>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
