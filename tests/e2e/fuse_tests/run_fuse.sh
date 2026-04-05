#!/bin/bash
# FUSE 故障注入测试运行脚本
# 
# 用法: ./run_fuse.sh [选项]
#
# 选项:
#   -v, --verbose      详细输出
#   -k EXPRESSION      只运行匹配的测试
#   --debug            调试模式（保留日志）
#   --keep-mount       测试后保留挂载点（用于检查）
#   --no-cleanup       不清理测试数据
#   --help             显示帮助

set -e

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 默认配置
VERBOSE=""
PYTEST_ARGS=""
DEBUG=false
KEEP_MOUNT=false
NO_CLEANUP=false

# 解析参数
while [[ $# -gt 0 ]]; do
    case $1 in
        -v|--verbose)
            VERBOSE="-v"
            shift
            ;;
        -k)
            PYTEST_ARGS="$PYTEST_ARGS -k $2"
            shift 2
            ;;
        --debug)
            DEBUG=true
            shift
            ;;
        --keep-mount)
            KEEP_MOUNT=true
            export FUSE_KEEP_MOUNT=1
            shift
            ;;
        --no-cleanup)
            NO_CLEANUP=true
            export FUSE_NO_CLEANUP=1
            shift
            ;;
        --help)
            echo "FUSE 故障注入测试运行脚本"
            echo ""
            echo "用法: ./run_fuse.sh [选项]"
            echo ""
            echo "选项:"
            echo "  -v, --verbose      详细输出"
            echo "  -k EXPRESSION      只运行匹配的测试"
            echo "  --debug            调试模式（保留日志）"
            echo "  --keep-mount       测试后保留挂载点"
            echo "  --no-cleanup       不清理测试数据"
            echo "  --help             显示帮助"
            echo ""
            echo "示例:"
            echo "  ./run_fuse.sh -v                    # 详细输出"
            echo "  ./run_fuse.sh -k test_import        # 只运行 import 测试"
            echo "  ./run_fuse.sh --debug --keep-mount  # 调试，保留挂载点"
            exit 0
            ;;
        *)
            echo "未知选项: $1"
            echo "使用 --help 查看帮助"
            exit 1
            ;;
    esac
done

echo -e "${GREEN}=== Svault FUSE 故障注入测试 ===${NC}"
echo ""
echo "测试类别:"
echo "  - test_import_fuse.py: 导入中断场景"
echo "  - test_recheck_fuse.py: 校验中断场景"
echo "  - test_verify_fuse.py: 验证中断场景"
echo "  - test_corruption_fuse.py: 硬件损坏/静默损坏模拟"
echo ""

# 检查环境
echo -e "${YELLOW}检查环境...${NC}"

# 检查 Python
if ! command -v python3 &> /dev/null; then
    echo -e "${RED}错误: 未找到 python3${NC}"
    exit 1
fi

# 检查 FUSE 库
if python3 -c "import fuse" 2>/dev/null; then
    echo "  ✓ fusepy 已安装"
elif python3 -c "import pyfuse3" 2>/dev/null; then
    echo "  ✓ pyfuse3 已安装"
else
    echo -e "${YELLOW}  ⚠ 未找到 FUSE 库，尝试安装 fusepy...${NC}"
    pip install fusepy || {
        echo -e "${RED}错误: 无法安装 fusepy${NC}"
        echo "请手动安装: pip install fusepy"
        exit 1
    }
fi

# 检查 FUSE 设备
if [[ -e /dev/fuse ]]; then
    echo "  ✓ /dev/fuse 存在"
else
    echo -e "${RED}错误: /dev/fuse 不存在，FUSE 未安装${NC}"
    exit 1
fi

# 检查用户权限
if [[ $EUID -eq 0 ]]; then
    echo "  ✓ 以 root 运行"
elif groups | grep -qE '\bfuse\b'; then
    echo "  ✓ 用户在 fuse 组"
else
    echo -e "${YELLOW}  ⚠ 可能无 FUSE 权限，尝试继续...${NC}"
fi

echo ""

# 检查 svault 二进制
E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$E2E_DIR/../.." && pwd)"
SVALUT_DEBUG="$PROJECT_ROOT/target/debug/svault"
SVALUT_RELEASE="$PROJECT_ROOT/target/release/svault"

if [[ -x "$SVALUT_DEBUG" ]]; then
    echo "  ✓ 使用 debug 构建: $SVALUT_DEBUG"
    export SVALUT_BIN="$SVALUT_DEBUG"
elif [[ -x "$SVALUT_RELEASE" ]]; then
    echo "  ✓ 使用 release 构建: $SVALUT_RELEASE"
    export SVALUT_BIN="$SVALUT_RELEASE"
else
    echo -e "${YELLOW}  ⚠ 未找到 svault 二进制，尝试构建...${NC}"
    (cd "$PROJECT_ROOT" && cargo build -p svault) || {
        echo -e "${RED}错误: 构建失败${NC}"
        exit 1
    }
    export SVALUT_BIN="$SVALUT_DEBUG"
fi

echo ""

# 设置 Python 路径
export PYTHONPATH="$E2E_DIR:$E2E_DIR/..:$PYTHONPATH"

# 运行测试
echo -e "${GREEN}运行 FUSE 测试...${NC}"
echo ""

PYTEST_CMD="python3 -m pytest $VERBOSE $PYTEST_ARGS"

if $DEBUG; then
    PYTEST_CMD="$PYTEST_CMD --tb=long -s"
    export FUSE_DEBUG=1
else
    PYTEST_CMD="$PYTEST_CMD --tb=short"
fi

if $NO_CLEANUP; then
    PYTEST_CMD="$PYTEST_CMD --no-cleanup"
fi

echo "命令: $PYTEST_CMD"
echo ""

cd "$E2E_DIR"
$PYTEST_CMD || {
    echo ""
    echo -e "${RED}测试失败${NC}"
    
    if $KEEP_MOUNT; then
        echo ""
        echo "挂载点保留在: /tmp/svault-fuse-*"
        echo "检查命令: ls -la /tmp/svault-fuse-*/"
    fi
    
    exit 1
}

echo ""
echo -e "${GREEN}所有 FUSE 测试通过!${NC}"

if $KEEP_MOUNT; then
    echo ""
    echo -e "${YELLOW}挂载点保留在: /tmp/svault-fuse-*/${NC}"
    echo "手动卸载: fusermount -u /tmp/svault-fuse-<name>"
fi
