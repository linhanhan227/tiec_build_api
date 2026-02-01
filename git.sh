#!/bin/bash
#set -euo pipefail

# 配置颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # 无颜色

# 全局变量
current_branch=""
remote_branch=""
commit_message=""
skip_commit=false

# 工具函数：信息提示
info() {
    echo -e "\n${BLUE}=== $1 ===${NC}"
}

# 工具函数：成功提示
success() {
    echo -e "${GREEN}✅ $1${NC}"
}

# 工具函数：警告提示
warning() {
    echo -e "${YELLOW}⚠️ $1${NC}"
}

# 工具函数：错误提示并返回菜单
error_return() {
    echo -e "${RED}❌ 错误：$1${NC}"
    read -p "按Enter键返回菜单..."
}

# 工具函数：错误提示并退出
error_exit() {
    echo -e "${RED}❌ 错误：$1${NC}"
    exit 1
}

# 1. 检查Git是否安装
check_git_installed() {
    info "检查Git环境"
    if ! command -v git &> /dev/null; then
        error_exit "未检测到Git工具，请先安装Git再执行脚本"
    fi
    success "Git已安装"
    read -p "按Enter键返回菜单..."
}

# 2. 初始化Git仓库
init_repo() {
    info "仓库初始化"
    if [ ! -d ".git" ]; then
        read -p "未检测到Git仓库，是否初始化？(y/n)：" init_confirm
        if [[ "$init_confirm" =~ ^[Yy]$ ]]; then
            echo "执行: git init"
            if git init; then
                success "Git仓库初始化完成"
            else
                error_return "仓库初始化失败"
                return 1
            fi
        else
            echo "取消初始化"
        fi
    else
        warning "已检测到Git仓库，无需初始化"
    fi
    read -p "按Enter键返回菜单..."
}

# 3. 配置远程仓库
config_remote() {
    info "远程仓库配置"
    read -p "请输入远程仓库URL（例：https://github.com/your-name/your-repo.git）：" remote_url

    # 验证URL格式
    if ! [[ "$remote_url" =~ ^(https?://|git@) ]]; then
        error_return "远程URL格式无效（需以https://或git@开头）"
        return 1
    fi

    # 处理已有远程配置
    if git remote | grep -q "origin"; then
        current_remote=$(git remote get-url origin)
        echo "当前origin远程地址：$current_remote"
        read -p "是否覆盖现有origin？(y/n)：" overwrite_confirm
        
        if [[ "$overwrite_confirm" =~ ^[Yy]$ ]]; then
            echo "执行: git remote rm origin"
            git remote rm origin
            echo "执行: git remote add origin $remote_url"
            if git remote add origin "$remote_url"; then
                success "远程仓库已更新"
            else
                error_return "添加远程仓库失败"
                return 1
            fi
        else
            echo "保留现有远程配置"
        fi
    else
        echo "执行: git remote add origin $remote_url"
        if git remote add origin "$remote_url"; then
            success "远程仓库已添加"
        else
            error_return "添加远程仓库失败"
            return 1
        fi
    fi

    # 验证远程连接
    echo -n "验证远程仓库连通性..."
    if ! git ls-remote --exit-code origin &> /dev/null; then
        error_return "远程仓库连接失败（可能URL错误、无访问权限或网络问题）"
        return 1
    fi
    success "远程仓库连接验证成功"
    read -p "按Enter键返回菜单..."
}

# 4. 检查工作区状态
check_workspace() {
    info "工作区状态检查"
    has_changes=false
    if ! git diff --quiet --cached --exit-code; then
        has_changes=true
    elif [ -n "$(git ls-files --others --exclude-standard)" ]; then
        has_changes=true
    fi

    if [ "$has_changes" = false ]; then
        warning "工作区无修改文件"
        skip_commit=true
    else
        warning "工作区有修改文件"
        git status --short
        skip_commit=false
    fi
    read -p "按Enter键返回菜单..."
}

# 5. 添加文件到暂存区
add_files() {
    info "文件暂存操作"
    git status --short
    
    read -p "是否添加所有修改文件到暂存区？(y/n)：" add_confirm
    if [[ "$add_confirm" =~ ^[Yy]$ ]]; then
        # 检查.gitignore文件
        if [ ! -f ".gitignore" ]; then
            warning "未检测到.gitignore文件，可能导致敏感文件被提交！"
            read -p "是否继续添加所有文件？(y/n)：" ignore_confirm
            if ! [[ "$ignore_confirm" =~ ^[Yy]$ ]]; then
                echo "请先创建.gitignore文件配置忽略规则"
                read -p "按Enter键返回菜单..."
                return 1
            fi
        fi
        echo "执行: git add ."
        if git add .; then
            success "所有文件已添加到暂存区"
        else
            error_return "添加文件失败"
            return 1
        fi
    else
        read -p "请输入需要添加的文件/目录（空格分隔）：" add_files
        if [ -n "$add_files" ]; then
            echo "执行: git add $add_files"
            if git add $add_files; then
                success "指定文件已添加到暂存区"
            else
                error_return "添加指定文件失败"
                return 1
            fi
        else
            echo "未添加任何文件"
        fi
    fi
    read -p "按Enter键返回菜单..."
}

# 6. 输入提交信息并提交更改
commit_changes() {
    info "提交变更"
    
    # 检查是否有需要提交的内容
    if git diff --cached --quiet; then
        error_return "没有需要提交的暂存文件，请先添加文件到暂存区"
        return 1
    fi

    # 输入提交信息
    echo "提交信息规范提示："
    echo "  <类型>[可选作用域]: <描述>"
    echo "  例：feat(login): 增加验证码登录功能"
    echo "  类型：feat(新功能)、fix(修复)、docs(文档)、style(格式)、refactor(重构)、test(测试)、chore(构建)"

    while true; do
        read -p "请输入提交信息：" commit_message
        if [ -n "$commit_message" ] && [ ${#commit_message} -ge 5 ]; then
            break
        fi
        warning "提交信息不能为空且长度需≥5个字符，请重新输入"
    done

    # 确认提交
    echo "即将提交的变更："
    git diff --cached --name-status
    
    read -p "确认提交以上变更？(y/n)：" commit_confirm
    if [[ "$commit_confirm" =~ ^[Yy]$ ]]; then
        echo "执行: git commit -m '$commit_message'"
        if git commit -m "$commit_message"; then
            success "本地提交完成"
        else
            error_return "提交失败"
            return 1
        fi
    else
        echo "取消提交"
    fi
    read -p "按Enter键返回菜单..."
}

# 7. 合并分支
merge_branch() {
    info "合并分支操作"
    # 获取当前分支
    current_branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || :)
    if [ -z "${current_branch:-}" ]; then
        error_return "无法获取当前分支名称，可能仓库未初始化或没有提交记录"
        return 1
    fi
    echo "当前分支：$current_branch"

    # 显示可用分支列表
    echo -e "\n${YELLOW}本地分支：${NC}"
    git branch
    echo -e "\n${YELLOW}远程分支：${NC}"
    git branch -r

    # 选择要合并的源分支
    read -p "请输入要合并到当前分支的源分支名称：" source_branch
    if [ -z "$source_branch" ]; then
        error_return "源分支名称不能为空"
        return 1
    fi

    # 检查源分支是否存在
    if ! git show-ref --verify --quiet "refs/heads/$source_branch" && ! git show-ref --verify --quiet "refs/remotes/origin/$source_branch"; then
        error_return "分支 $source_branch 不存在"
        return 1
    fi

    # 如果是远程分支，先拉取最新代码
    if ! git show-ref --verify --quiet "refs/heads/$source_branch" && git show-ref --verify --quiet "refs/remotes/origin/$source_branch"; then
        echo "执行: git fetch origin $source_branch (获取远程分支最新更新)"
        if ! git fetch origin "$source_branch"; then
            error_return "获取远程分支更新失败"
            return 1
        fi
        source_branch="origin/$source_branch"
    fi

    # 执行合并
    echo "执行: git merge $source_branch (合并分支)"
    if ! git merge "$source_branch"; then
        # 如果失败，尝试允许合并无关历史
        warning "合并失败，尝试允许合并无关历史..."
        if ! git merge "$source_branch" --allow-unrelated-histories; then
            error_return "合并分支时发生冲突，请手动解决冲突后再试"
            return 1
        fi
    fi
    
    success "分支合并完成"
    read -p "按Enter键返回菜单..."
}

# 8. 推送至远程仓库
push_to_remote() {
    info "远程推送操作"
    # 获取当前分支
    current_branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || :)
    if [ -z "${current_branch:-}" ]; then
        error_return "无法获取当前分支名称，可能仓库未初始化或没有提交记录"
        return 1
    fi
    echo "当前本地分支：$current_branch"

    # 显示远程分支列表
    echo -e "\n${YELLOW}远程分支列表：${NC}"
    git ls-remote --heads origin | awk -F'/' '{print $3}' | sort | uniq

    # 选择要推送的远程分支
    read -p "请输入要推送到的远程分支名称（默认: $current_branch）：" remote_branch
    remote_branch=${remote_branch:-$current_branch}  # 使用默认值

    # 询问是否强制推送
    force_push=""
    read -p "是否使用强制推送? 警告: 强制推送可能覆盖远程更改! (y/n)：" force_confirm
    if [[ "$force_confirm" =~ ^[Yy]$ ]]; then
        force_push="-f"
        warning "已启用强制推送模式，请确保您知道自己在做什么！"
    fi

    # 构建推送命令
    push_cmd=""
    if ! git ls-remote --exit-code --heads origin "${remote_branch}" &> /dev/null; then
        warning "远程仓库不存在$remote_branch分支，将执行首次推送（自动关联分支）"
        push_cmd="git push $force_push -u origin $current_branch:$remote_branch"
    else
        push_cmd="git push $force_push origin $current_branch:$remote_branch"
    fi

    # 确认并执行推送
    read -p "是否执行推送：$push_cmd？(y/n)：" push_confirm
    if [[ "$push_confirm" =~ ^[Yy]$ ]]; then
        echo "执行: $push_cmd"
        if ! $push_cmd; then
            error_return "推送失败！可能原因：远程有新提交、无权限或网络问题"
            return 1
        fi
        success "远程推送完成"
    else
        echo "取消推送"
    fi
    read -p "按Enter键返回菜单..."
}

# 9. 切换分支
switch_branch() {
    info "切换分支操作"
    # 获取当前分支
    current_branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || :)
    if [ -n "${current_branch:-}" ]; then
        echo "当前分支：$current_branch"
    else
        warning "当前没有处于任何分支上"
    fi

    # 显示可用分支列表
    echo -e "\n${YELLOW}本地分支：${NC}"
    git branch
    echo -e "\n${YELLOW}远程分支：${NC}"
    git branch -r

    # 输入要切换的分支名称
    read -p "请输入要切换到的分支名称：" target_branch
    if [ -z "$target_branch" ]; then
        error_return "分支名称不能为空"
        return 1
    fi

    # 检查本地分支是否存在
    if git show-ref --verify --quiet "refs/heads/$target_branch"; then
        # 切换到本地分支
        echo "执行: git checkout $target_branch"
        if git checkout "$target_branch"; then
            success "已切换到本地分支 $target_branch"
        else
            error_return "切换分支失败"
            return 1
        fi
    else
        # 检查远程分支是否存在
        if git show-ref --verify --quiet "refs/remotes/origin/$target_branch"; then
            # 拉取并切换到远程分支
            echo "执行: git checkout -b $target_branch origin/$target_branch"
            if git checkout -b "$target_branch" "origin/$target_branch"; then
                success "已从远程拉取并切换到分支 $target_branch"
            else
                error_return "拉取并切换分支失败"
                return 1
            fi
        else
            # 分支不存在，询问是否创建新分支
            read -p "分支 $target_branch 不存在，是否创建新分支？(y/n)：" create_confirm
            if [[ "$create_confirm" =~ ^[Yy]$ ]]; then
                echo "执行: git checkout -b $target_branch"
                if git checkout -b "$target_branch"; then
                    success "已创建并切换到新分支 $target_branch"
                else
                    error_return "创建并切换分支失败"
                    return 1
                fi
            else
                echo "取消切换分支"
            fi
        fi
    fi
    read -p "按Enter键返回菜单..."
}

# 10. 查看分支信息
show_branches() {
    info "分支信息"
    echo -e "${YELLOW}本地分支：${NC}"
    git branch
    echo -e "\n${YELLOW}远程分支：${NC}"
    git branch -r
    echo -e "\n${YELLOW}当前分支：${NC}$(git rev-parse --abbrev-ref HEAD 2>/dev/null || :)"
    read -p "按Enter键返回菜单..."
}

# 11. 强制提交并推送
force_commit_push() {
    info "强制提交并推送"
    
    # 警告用户此操作的危险性
    warning "警告：此操作将添加所有未忽略的更改、创建提交并强制推送到远程仓库！"
    warning "这可能会覆盖远程仓库的更改，仅在您确认清楚后果时使用！"
    read -p "是否继续？(y/N)：" confirm
    if ! [[ "$confirm" =~ ^[Yy]$ ]]; then
        echo "已取消强制提交推送操作"
        read -p "按Enter键返回菜单..."
        return 0
    fi

    # 检查是否为Git仓库
    if [ ! -d ".git" ]; then
        error_return "当前目录不是Git仓库，请先初始化仓库"
        return 1
    fi

    # 添加所有未忽略的文件到暂存区（尊重.gitignore规则）
    echo "执行: git add ."
    if ! git add .; then
        error_return "添加文件失败"
        return 1
    fi

    # 检查是否有需要提交的内容
    if git diff --cached --quiet; then
        warning "没有需要提交的变更，无需提交"
    else
        # 获取提交信息
        read -p "请输入提交信息（默认：'强制提交: 自动同步更改'）：" commit_msg
        commit_msg=${commit_msg:-"强制提交: 自动同步更改"}
        
        # 强制提交
        echo "执行: git commit -m '$commit_msg' --allow-empty"
        if ! git commit -m "$commit_msg" --allow-empty; then
            error_return "强制提交失败"
            return 1
        fi
        success "本地强制提交完成"
    fi

    # 获取当前分支
    current_branch=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || :)
    if [ -z "${current_branch:-}" ]; then
        error_return "无法获取当前分支名称"
        return 1
    fi
    echo "当前分支：$current_branch"

    # 选择远程分支（默认与当前分支同名）
    read -p "请输入要推送到的远程分支名称（默认: $current_branch）：" remote_branch
    remote_branch=${remote_branch:-$current_branch}

    # 构建强制推送命令
    push_cmd=""
    if ! git ls-remote --exit-code --heads origin "${remote_branch}" &> /dev/null; then
        warning "远程仓库不存在$remote_branch分支，将执行首次强制推送（自动关联分支）"
        push_cmd="git push -f -u origin $current_branch:$remote_branch"
    else
        push_cmd="git push -f origin $current_branch:$remote_branch"
    fi

    # 再次确认强制推送
    warning "即将执行强制推送: $push_cmd"
    read -p "这将覆盖远程分支的历史记录，确定要执行吗？(y/N)：" push_confirm
    if [[ "$push_confirm" =~ ^[Yy]$ ]]; then
        echo "执行: $push_cmd"
        if ! $push_cmd; then
            error_return "强制推送失败"
            return 1
        fi
        success "强制推送完成"
    else
        echo "已取消强制推送"
    fi

    read -p "按Enter键返回菜单..."
}

# 显示主菜单
show_menu() {
    clear
    echo -e "\n${BLUE}=== Git仓库管理工具 ===${NC}"
    echo "1. 检查Git是否安装"
    echo "2. 初始化Git仓库"
    echo "3. 配置远程仓库"
    echo "4. 检查工作区状态"
    echo "5. 添加文件到暂存区"
    echo "6. 提交更改"
    echo "7. 合并分支"
    echo "8. 推送至远程仓库"
    echo "9. 切换分支"
    echo "10. 查看分支信息"
    echo "11. 强制提交并推送"
    echo "0. 退出脚本"
    echo -e "\n请输入选项 [0-11]："
}

# 主程序
check_git_installed  # 启动时先检查Git是否安装

while true; do
    show_menu
    read -p "选择操作：" choice
    case $choice in
        1) check_git_installed ;;
        2) init_repo ;;
        3) config_remote ;;
        4) check_workspace ;;
        5) add_files ;;
        6) commit_changes ;;
        7) merge_branch ;;
        8) push_to_remote ;;
        9) switch_branch ;;
        10) show_branches ;;
        11) force_commit_push ;;
        0) 
            echo -e "\n${GREEN}感谢使用，再见！${NC}"
            exit 0 
            ;;
        *) 
            echo -e "${RED}无效选项，请输入0-11之间的数字${NC}"
            read -p "按Enter键继续..."
            ;;
    esac
done
