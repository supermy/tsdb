CC = gcc
CFLAGS = -Wall -Wextra -std=c99 -O2 -g
LDFLAGS = -lm

ROCKSDB_HOME = /usr/local/opt/rocksdb
LDFLAGS += -L$(ROCKSDB_HOME)/lib -lrocksdb -lpthread -ldl

SRCDIR = .
OBJDIR = obj

SOURCES = rtsdb_types.c rtsdb_config.c rtsdb_storage.c rtsdb_index.c rtsdb_query.c rtsdb.c
OBJECTS = $(SOURCES:%.c=$(OBJDIR)/%.o)

LIB = librtsdb.a
EXAMPLE = example

.PHONY: all clean

all: $(OBJDIR) $(LIB) $(EXAMPLE)

$(OBJDIR):
	mkdir -p $(OBJDIR)

$(OBJDIR)/%.o: $(SRCDIR)/%.c
	$(CC) $(CFLAGS) -I$(ROCKSDB_HOME)/include -c $< -o $@

$(LIB): $(OBJECTS)
	ar rcs $@ $^

$(EXAMPLE): example.c $(LIB)
	$(CC) $(CFLAGS) example.c -I. -L. -lrtsdb $(LDFLAGS) -o $@

clean:
	rm -rf $(OBJDIR) $(LIB) $(EXAMPLE) rtsdb_data
