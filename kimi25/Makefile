CC = gcc
CFLAGS = -Wall -Wextra -std=c99 -O2 -g
LDFLAGS = -lm

SRC = kimi_tsdb.c
OBJ = $(SRC:.c=.o)
LIB = libkimi_tsdb.a

EXAMPLE_SRC = example.c
EXAMPLE = example

.PHONY: all clean

all: $(LIB) $(EXAMPLE)

$(OBJ): $(SRC)
	$(CC) $(CFLAGS) -c $< -o $@

$(LIB): $(OBJ)
	ar rcs $@ $^

$(EXAMPLE): $(EXAMPLE_SRC) $(LIB)
	$(CC) $(CFLAGS) $< -L. -lkimi_tsdb $(LDFLAGS) -o $@

clean:
	rm -f $(OBJ) $(LIB) $(EXAMPLE)
	rm -rf kimi_data
