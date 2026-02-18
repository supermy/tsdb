CC = gcc
CFLAGS = -Wall -Wextra -std=c99 -O2 -g
LDFLAGS = -lm

SRCDIR = .
OBJDIR = obj
BINDIR = bin

SOURCES = tsdb_types.c tsdb_config.c tsdb_storage.c tsdb_index.c \
          tsdb_query.c tsdb_compress.c tsdb_cache.c tsdb.c

OBJECTS = $(SOURCES:%.c=$(OBJDIR)/%.o)

LIB = libtsdb.a

EXAMPLE = example

.PHONY: all clean

all: $(OBJDIR) $(LIB) $(EXAMPLE)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -c $< -o $@

$(LIB): $(OBJECTS)
	ar rcs $@ $^

$(EXAMPLE): $(LIB) example.c
	$(CC) $(CFLAGS) example.c -L. -ltsdb $(LDFLAGS) -o $@

clean:
	rm -rf $(OBJDIR) $(LIB) $(EXAMPLE)
