CC = gcc
CFLAGS = -Wall -Wextra -std=c99 -O2 -g
LDFLAGS = -lm

SRCDIR = .
OBJDIR = obj

SOURCES = tsdb25_types.c tsdb25_config.c tsdb25_storage.c tsdb25_index.c \
          tsdb25_query.c tsdb25_cache.c tsdb25_wal.c tsdb25.c

OBJECTS = $(SOURCES:%.c=$(OBJDIR)/%.o)

LIB = libtsdb25.a

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
	$(CC) $(CFLAGS) example.c -L. -ltsdb25 $(LDFLAGS) -o $@

clean:
	rm -rf $(OBJDIR) $(LIB) $(EXAMPLE) tsdb25_data
