PROJECT_DIR=$(dirname $BASH_SOURCE)
PROJECT_FILES=$(echo $(find $PROJECT_DIR/riscv-programs -name "*.h") $(find $PROJECT_DIR/riscv-programs -name "*.c") $(find $PROJECT_DIR/riscv-programs -name "*.asm") $(find $PROJECT_DIR/src -name "*.rs"))

emacs $PROJECT_FILES
