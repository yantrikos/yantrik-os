#!/usr/bin/env python3
"""QLoRA fine-tuning of Qwen3.5-4B for Yantrik companion.

Uses unsloth for faster training on single GPU (RTX 3090 Ti 24GB).
Run: C:/Python312/python.exe training/train_qlora.py
"""

import argparse
import os
import json

os.environ["TORCHDYNAMO_DISABLE"] = "1"
os.environ["PYTORCH_CUDA_ALLOC_CONF"] = "expandable_segments:True"
os.environ["CUDA_VISIBLE_DEVICES"] = "0"  # Use GPU 0 only

# ── Config ──────────────────────────────────────────────────────────────────
MODEL_NAME = "Qwen/Qwen3.5-4B"
OUTPUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "checkpoints")
DATA_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "output")

LORA_R = 16
LORA_ALPHA = 16
MAX_SEQ_LENGTH = 2048
SEED = 3407

BATCH_SIZE = 4             # Per-device batch (bf16 4B model fits with batch=4)
GRAD_ACCUM_STEPS = 4       # Effective batch = 4 * 4 = 16
LEARNING_RATE = 2e-4
NUM_EPOCHS = 3
WARMUP_STEPS = 50
LOGGING_STEPS = 10
SAVE_STEPS = 500
EVAL_STEPS = 500

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
    args = parser.parse_args()

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    from unsloth import FastLanguageModel
    from trl import SFTTrainer, SFTConfig
    from datasets import Dataset

    print(f"\n{'='*60}")
    print(f"Loading {MODEL_NAME} in bf16...")
    print(f"{'='*60}\n")

    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=MODEL_NAME,
        max_seq_length=args.max_seq_len,
        load_in_4bit=False,
        load_in_16bit=True,
        full_finetuning=False,
    )

    # Qwen3.5 returns VL processor — extract inner tokenizer
    if hasattr(tokenizer, 'tokenizer'):
        print(f"Extracting inner tokenizer from {type(tokenizer).__name__}")
        text_tokenizer = tokenizer.tokenizer
        if hasattr(tokenizer, 'chat_template') and not hasattr(text_tokenizer, 'chat_template'):
            text_tokenizer.chat_template = tokenizer.chat_template
        tokenizer = text_tokenizer

    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    print("Adding LoRA adapters...")
    model = FastLanguageModel.get_peft_model(
        model,
        r=LORA_R,
        lora_alpha=LORA_ALPHA,
        lora_dropout=0.0,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj",
                         "gate_proj", "up_proj", "down_proj"],
        bias="none",
        use_gradient_checkpointing="unsloth",
        random_state=SEED,
        max_seq_length=args.max_seq_len,
    )

    trainable = sum(p.numel() for p in model.parameters() if p.requires_grad)
    total = sum(p.numel() for p in model.parameters())
    print(f"Trainable: {trainable:,} / {total:,} ({100*trainable/total:.2f}%)")

    # ── Load and format data ────────────────────────────────────────────
    def read_jsonl(path):
        examples = []
        with open(path, "r", encoding="utf-8") as f:
            for line in f:
                if line.strip():
                    examples.append(json.loads(line))
        return examples

    train_path = os.path.join(DATA_DIR, "train_sft.jsonl")
    eval_path = os.path.join(DATA_DIR, "eval_sft.jsonl")

    train_dataset = Dataset.from_list(read_jsonl(train_path))
    eval_dataset = Dataset.from_list(read_jsonl(eval_path))
    print(f"Train: {len(train_dataset)}, Eval: {len(eval_dataset)}")

    def format_example(example):
        convos = example["conversations"]
        messages = [
            {"role": ROLE_MAP.get(t["from"], t["from"]), "content": t["value"]}
            for t in convos
        ]
        text = tokenizer.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=False
        )
        return {"text": text}

    print("Applying chat template...")
    train_dataset = train_dataset.map(format_example, remove_columns=["conversations"], num_proc=1)
    eval_dataset = eval_dataset.map(format_example, remove_columns=["conversations"], num_proc=1)

    # ── Train ───────────────────────────────────────────────────────────
    effective_batch = args.batch_size * GRAD_ACCUM_STEPS
    total_steps = (len(train_dataset) * args.epochs) // effective_batch
    print(f"\nBatch: {args.batch_size} x {GRAD_ACCUM_STEPS} = {effective_batch}")
    print(f"Total steps: {total_steps}, ETA: ~{total_steps * 5 / 3600:.1f}h\n")

    trainer = SFTTrainer(
        model=model,
        tokenizer=tokenizer,
        train_dataset=train_dataset,
        eval_dataset=eval_dataset,
        args=SFTConfig(
            max_seq_length=args.max_seq_len,
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
            optim="adamw_8bit",
            seed=SEED,
            output_dir=OUTPUT_DIR,
            report_to="none",
            dataset_num_proc=1,
            dataloader_num_workers=0,
            average_tokens_across_devices=False,  # Fix unsloth int.mean() bug
            bf16=True,
            fp16=False,
        ),
    )

    print(f"{'='*60}")
    print("Starting training...")
    print(f"{'='*60}\n")

    resume_from = args.resume
    if resume_from and not os.path.isdir(resume_from):
        print(f"Warning: checkpoint {resume_from} not found")
        resume_from = None

    stats = trainer.train(resume_from_checkpoint=resume_from)

    print(f"\n{'='*60}")
    print("Training complete!")
    print(f"{'='*60}")
    print(f"  Train loss:  {stats.training_loss:.4f}")
    print(f"  Runtime:     {stats.metrics['train_runtime']:.1f}s")
    print(f"  Samples/s:   {stats.metrics['train_samples_per_second']:.1f}")

    lora_dir = os.path.join(OUTPUT_DIR, "final_lora")
    print(f"\nSaving LoRA adapter to {lora_dir}...")
    model.save_pretrained(lora_dir)
    tokenizer.save_pretrained(lora_dir)

    print("\nRunning evaluation...")
    eval_results = trainer.evaluate()
    print(f"  Eval loss: {eval_results['eval_loss']:.4f}")

    if args.merge or args.gguf:
        merge_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "model_merged")
        os.makedirs(merge_dir, exist_ok=True)
        print(f"\nMerging LoRA into base model -> {merge_dir}")
        model.save_pretrained_merged(merge_dir, tokenizer, save_method="merged_16bit")

    if args.gguf:
        gguf_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "model_gguf")
        os.makedirs(gguf_dir, exist_ok=True)
        print(f"Exporting GGUF -> {gguf_dir}")
        model.save_pretrained_gguf(gguf_dir, tokenizer, quantization_method="q4_k_m")

    print("\nDone!")


if __name__ == "__main__":
    main()
