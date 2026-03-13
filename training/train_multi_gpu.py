#!/usr/bin/env python3
"""Multi-GPU fine-tuning of Qwen3.5-4B for Yantrik companion.

No unsloth dependency — uses HuggingFace PEFT + Transformers directly.
Splits model across 2 GPUs via device_map="balanced" (model parallelism).

Run: C:/Python312/python.exe training/train_multi_gpu.py
"""

import argparse
import os
import json
import time

os.environ["TORCHDYNAMO_DISABLE"] = "1"
os.environ["PYTORCH_CUDA_ALLOC_CONF"] = "expandable_segments:True"

# ── Config ──────────────────────────────────────────────────────────────────
MODEL_NAME = "Qwen/Qwen3.5-4B"
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
OUTPUT_DIR = os.path.join(SCRIPT_DIR, "checkpoints")
DATA_DIR = os.path.join(SCRIPT_DIR, "output")

LORA_R = 16
LORA_ALPHA = 16
MAX_SEQ_LENGTH = 1024
SEED = 3407

BATCH_SIZE = 8
GRAD_ACCUM_STEPS = 4       # Effective batch = 8 * 4 = 32
LEARNING_RATE = 2e-4
NUM_EPOCHS = 3
WARMUP_STEPS = 50
LOGGING_STEPS = 10
SAVE_STEPS = 200
EVAL_STEPS = 200

ROLE_MAP = {"system": "system", "human": "user", "gpt": "assistant", "tool": "user"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--resume", type=str, default=None)
    parser.add_argument("--epochs", type=int, default=NUM_EPOCHS)
    parser.add_argument("--batch-size", type=int, default=BATCH_SIZE)
    parser.add_argument("--lr", type=float, default=LEARNING_RATE)
    parser.add_argument("--max-seq-len", type=int, default=MAX_SEQ_LENGTH)
    parser.add_argument("--merge", action="store_true")
    parser.add_argument("--gguf", action="store_true")
    parser.add_argument("--single-gpu", action="store_true", help="Force single GPU")
    args = parser.parse_args()

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, TrainingArguments, Trainer
    from peft import LoraConfig, get_peft_model, TaskType
    from datasets import Dataset

    n_gpus = torch.cuda.device_count()
    use_multi = n_gpus >= 2 and not args.single_gpu

    print(f"\n{'='*60}")
    print(f"Loading {MODEL_NAME} in bf16...")
    print(f"GPUs: {n_gpus}, Mode: {'model-parallel (balanced)' if use_multi else 'single GPU'}")
    print(f"{'='*60}\n")

    load_kwargs = {
        "pretrained_model_name_or_path": MODEL_NAME,
        "torch_dtype": torch.bfloat16,
        "trust_remote_code": True,
    }
    if use_multi:
        load_kwargs["device_map"] = "balanced"
    else:
        load_kwargs["device_map"] = {"": 0}

    model = AutoModelForCausalLM.from_pretrained(**load_kwargs)

    tokenizer = AutoTokenizer.from_pretrained(MODEL_NAME, trust_remote_code=True)
    # Qwen3.5 may return VL processor
    if hasattr(tokenizer, 'tokenizer'):
        print(f"Extracting inner tokenizer from {type(tokenizer).__name__}")
        text_tokenizer = tokenizer.tokenizer
        if hasattr(tokenizer, 'chat_template') and not hasattr(text_tokenizer, 'chat_template'):
            text_tokenizer.chat_template = tokenizer.chat_template
        tokenizer = text_tokenizer

    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    # Show device distribution
    devices = set()
    for name, param in model.named_parameters():
        devices.add(str(param.device))
    print(f"Model distributed across: {sorted(devices)}")

    # ── LoRA ─────────────────────────────────────────────────────────────
    print("Adding LoRA adapters...")
    # Enable gradient checkpointing before LoRA
    model.gradient_checkpointing_enable()

    lora_config = LoraConfig(
        r=LORA_R,
        lora_alpha=LORA_ALPHA,
        lora_dropout=0.0,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj",
                         "gate_proj", "up_proj", "down_proj"],
        bias="none",
        task_type=TaskType.CAUSAL_LM,
    )
    model = get_peft_model(model, lora_config)
    model.print_trainable_parameters()

    # ── Load and format data ─────────────────────────────────────────────
    def read_jsonl(path):
        examples = []
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    examples.append(json.loads(line))
        return examples

    train_path = os.path.join(DATA_DIR, "train_sft.jsonl")
    eval_path = os.path.join(DATA_DIR, "eval_sft.jsonl")

    train_raw = read_jsonl(train_path)
    eval_raw = read_jsonl(eval_path)
    print(f"Train: {len(train_raw)}, Eval: {len(eval_raw)}")

    def format_and_tokenize(examples):
        """Format conversations and tokenize in one step."""
        all_input_ids = []
        all_attention_mask = []
        all_labels = []

        for convos in examples["conversations"]:
            messages = [
                {"role": ROLE_MAP.get(t["from"], t["from"]), "content": t["value"]}
                for t in convos
            ]
            text = tokenizer.apply_chat_template(
                messages, tokenize=False, add_generation_prompt=False
            )
            encoded = tokenizer(
                text,
                truncation=True,
                max_length=args.max_seq_len,
                padding="max_length",
                return_tensors=None,
            )
            all_input_ids.append(encoded["input_ids"])
            all_attention_mask.append(encoded["attention_mask"])
            # For causal LM, labels = input_ids (shifted internally by the model)
            labels = encoded["input_ids"].copy()
            # Mask padding tokens in labels
            labels = [-100 if token == tokenizer.pad_token_id else token for token in labels]
            all_labels.append(labels)

        return {
            "input_ids": all_input_ids,
            "attention_mask": all_attention_mask,
            "labels": all_labels,
        }

    print("Tokenizing dataset...")
    train_dataset = Dataset.from_list(train_raw)
    eval_dataset = Dataset.from_list(eval_raw)

    train_dataset = train_dataset.map(
        format_and_tokenize, batched=True, batch_size=100,
        remove_columns=["conversations"], num_proc=1,
    )
    eval_dataset = eval_dataset.map(
        format_and_tokenize, batched=True, batch_size=100,
        remove_columns=["conversations"], num_proc=1,
    )
    train_dataset.set_format("torch")
    eval_dataset.set_format("torch")

    # ── Train ────────────────────────────────────────────────────────────
    effective_batch = args.batch_size * GRAD_ACCUM_STEPS
    total_steps = (len(train_dataset) * args.epochs) // effective_batch
    print(f"\nBatch: {args.batch_size} x {GRAD_ACCUM_STEPS} = {effective_batch}")
    print(f"Total steps: {total_steps}\n")

    training_args = TrainingArguments(
        output_dir=OUTPUT_DIR,
        per_device_train_batch_size=args.batch_size,
        per_device_eval_batch_size=args.batch_size,
        gradient_accumulation_steps=GRAD_ACCUM_STEPS,
        learning_rate=args.lr,
        num_train_epochs=args.epochs,
        warmup_steps=WARMUP_STEPS,
        logging_steps=LOGGING_STEPS,
        save_steps=SAVE_STEPS,
        eval_strategy="steps",
        eval_steps=EVAL_STEPS,
        save_total_limit=3,
        load_best_model_at_end=True,
        metric_for_best_model="eval_loss",
        greater_is_better=False,
        optim="adamw_torch",  # Standard optimizer (no bitsandbytes dep)
        seed=SEED,
        report_to="none",
        dataloader_num_workers=0,
        bf16=True,
        fp16=False,
        # Disable DDP — we use model parallelism instead
        ddp_backend=None,
        remove_unused_columns=False,
    )

    trainer = Trainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=train_dataset,
        eval_dataset=eval_dataset,
        args=training_args,
    )

    print(f"{'='*60}")
    print("Starting training...")
    print(f"{'='*60}\n")

    resume_from = args.resume
    if resume_from and not os.path.isdir(resume_from):
        print(f"Warning: checkpoint {resume_from} not found")
        resume_from = None

    start_time = time.time()
    stats = trainer.train(resume_from_checkpoint=resume_from)
    elapsed = time.time() - start_time

    print(f"\n{'='*60}")
    print("Training complete!")
    print(f"{'='*60}")
    print(f"  Train loss:  {stats.training_loss:.4f}")
    print(f"  Runtime:     {elapsed:.1f}s ({elapsed/3600:.1f}h)")
    print(f"  Samples/s:   {stats.metrics['train_samples_per_second']:.1f}")

    lora_dir = os.path.join(OUTPUT_DIR, "final_lora")
    print(f"\nSaving LoRA adapter to {lora_dir}...")
    model.save_pretrained(lora_dir)
    tokenizer.save_pretrained(lora_dir)

    print("\nRunning evaluation...")
    eval_results = trainer.evaluate()
    print(f"  Eval loss: {eval_results['eval_loss']:.4f}")

    if args.merge or args.gguf:
        from peft import AutoPeftModelForCausalLM
        merge_dir = os.path.join(SCRIPT_DIR, "model_merged")
        os.makedirs(merge_dir, exist_ok=True)
        print(f"\nMerging LoRA into base model -> {merge_dir}")
        merged = model.merge_and_unload()
        merged.save_pretrained(merge_dir)
        tokenizer.save_pretrained(merge_dir)

    if args.gguf:
        print("\nTo export GGUF, run:")
        print(f"  python llama.cpp/convert_hf_to_gguf.py {merge_dir} --outtype q4_k_m")

    print("\nDone!")


if __name__ == "__main__":
    main()
