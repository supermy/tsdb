# Qwen35 TSDB Makefile

CC = gcc
CFLAGS = -Wall -Wextra -O3 -g -std=c99
LDFLAGS = -lrocksdb -lpthread -lm -lz -lzstd -lsnappy -lbz2 -llz4

# RocksDB 库路径（根据实际安装位置修改）
ROCKSDB_INCLUDE = /usr/local/include
ROCKSDB_LIB = /usr/local/lib

# 或者使用 brew 安装的路径（macOS）
# ROCKSDB_INCLUDE = /opt/homebrew/include
# ROCKSDB_LIB = /opt/homebrew/lib

INCLUDES = -I$(ROCKSDB_INCLUDE)
LIBRARIES = -L$(ROCKSDB_LIB) $(LDFLAGS)

# 目标文件
TARGET = qtsdb_example
LIB_TARGET = libqtsdb.a

# 源文件
SRCS = qtsdb.c
EXAMPLE_SRCS = example.c

# 对象文件
OBJS = $(SRCS:.c=.o)

.PHONY: all clean example lib install

all: $(TARGET)

# 编译库
lib: $(LIB_TARGET)

$(LIB_TARGET): $(OBJS)
	ar rcs $@ $^

# 编译示例
example: $(TARGET)

$(TARGET): $(EXAMPLE_SRCS) $(LIB_TARGET)
	$(CC) $(CFLAGS) $(INCLUDES) -o $@ $(EXAMPLE_SRCS) -L. -lqtsdb $(LIBRARIES)

# 编译对象
%.o: %.c qtsdb.h
	$(CC) $(CFLAGS) $(INCLUDES) -c $< -o $@

# 运行示例
run: $(TARGET)
	./$(TARGET)

# 安装
install: $(LIB_TARGET) qtsdb.h
	cp $(LIB_TARGET) /usr/local/lib/
	cp qtsdb.h /usr/local/include/

# 清理
clean:
	rm -f $(OBJS) $(TARGET) $(LIB_TARGET)
	rm -rf qwen35_data

# 性能测试
perf: $(TARGET)
	@echo "运行性能测试..."
	@./$(TARGET)
	@echo ""
	@du -sh qwen35_data

# 调试编译
debug: CFLAGS += -DDEBUG -g3
debug: clean all

# 帮助
help:
	@echo "Qwen35 TSDB Makefile"
	@echo ""
	@echo "目标:"
	@echo "  all      - 编译示例程序 (默认)"
	@echo "  lib      - 编译静态库"
	@echo "  example  - 编译示例程序"
	@echo "  run      - 编译并运行示例"
	@echo "  clean    - 清理编译文件"
	@echo "  install  - 安装库和头文件"
	@echo "  perf     - 运行性能测试"
	@echo "  debug    - 调试模式编译"
	@echo "  help     - 显示帮助信息"
