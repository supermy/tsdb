CC = gcc
CFLAGS = -Wall -Wextra -std=c99 -O2 -g
LDFLAGS = -lm

ROCKSDB_HOME = /usr/local/opt/rocksdb
LDFLAGS += -L$(ROCKSDB_HOME)/lib -lrocksdb -lpthread -ldl

SRC = rkimi_tsdb.c
OBJ = $(SRC:.c=.o)
LIB = librkimi_tsdb.a

EXAMPLE_SRC = example.c
EXAMPLE = example

.PHONY: all clean

all: $(LIB) $(EXAMPLE)

$(OBJ): $(SRC)
	$(CC) $(CFLAGS) -I$(ROCKSDB_HOME)/include -c $< -o $@

$(LIB): $(OBJ)
	ar rcs $@ $^

$(EXAMPLE): $(EXAMPLE_SRC) $(LIB)
	$(CC) $(CFLAGS) $(EXAMPLE_SRC) -I. -L. -lrkimi_tsdb $(LDFLAGS) -o $@

clean:
	rm -f $(OBJ) $(LIB) $(EXAMPLE) rkimi_data
